use crate::security::des;
// use crate::vnc::protocol::Message;
use crate::vnc::{protocol, zrle, Colour, Error, Rect, Result};
use byteorder::{BigEndian, ReadBytesExt};
use protocol::Message;
#[cfg(feature = "apple-auth")]
use security::apple_auth;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug)]
#[non_exhaustive]
pub enum AuthMethod {
    None,
    Password,
    AppleRemoteDesktop,
    /* more to come */
}

#[derive(Debug)]
#[non_exhaustive]
pub enum AuthChoice {
    None,
    Password([u8; 8]),
    AppleRemoteDesktop(String, String),
    /* more to come */
}

#[derive(Debug)]
pub enum Event {
    Disconnected(Option<Error>),
    Resize(u16, u16),
    SetColourMap {
        first_colour: u16,
        colours: Vec<Colour>,
    },
    PutPixels(Rect, Vec<u8>),
    CopyPixels {
        src: Rect,
        dst: Rect,
    },
    EndOfFrame,
    SetCursor {
        size: (u16, u16),
        hotspot: (u16, u16),
        pixels: Vec<u8>,
        mask_bits: Vec<u8>,
    },
    Clipboard(String),
    Bell,
}

impl Event {
    fn pump(
        mut stream: TcpStream,
        format: Arc<Mutex<protocol::PixelFormat>>,
        tx_events: &mut Sender<Event>,
    ) -> Result<()> {
        macro_rules! send {
            ($chan:expr, $data:expr) => {{
                match $chan.send($data) {
                    Ok(()) => (),
                    Err(_) => break,
                }
            }};
        }

        let mut zrle_decoder = zrle::Decoder::new();
        loop {
            let packet = match protocol::S2C::read_from(&mut stream) {
                Ok(packet) => packet,
                Err(Error::Disconnected) => {
                    send!(tx_events, Event::Disconnected(None));
                    break;
                }
                Err(error) => return Err(error),
            };
            debug!("<- {:?}", packet);

            let format = *format.lock().unwrap();
            match packet {
                protocol::S2C::SetColourMapEntries {
                    first_colour,
                    colours,
                } => {
                    send!(
                        tx_events,
                        Event::SetColourMap {
                            first_colour,
                            colours,
                        }
                    )
                }
                protocol::S2C::FramebufferUpdate { count } => {
                    for _ in 0..count {
                        let rectangle = protocol::Rectangle::read_from(&mut stream)?;
                        debug!("<- {:?}", rectangle);

                        let dst = Rect {
                            left: rectangle.x_position,
                            top: rectangle.y_position,
                            width: rectangle.width,
                            height: rectangle.height,
                        };
                        match rectangle.encoding {
                            protocol::Encoding::Raw => {
                                let length = (rectangle.width as usize)
                                    * (rectangle.height as usize)
                                    * (format.bits_per_pixel as usize / 8);
                                let mut pixels = vec![0; length];
                                // let mut pixels = Vec::with_capacity(length);
                                // unsafe { pixels.set_len(length as usize) }
                                stream.read_exact(&mut pixels)?;
                                debug!("<- ...pixels");
                                send!(tx_events, Event::PutPixels(dst, pixels))
                            }
                            protocol::Encoding::CopyRect => {
                                let copy_rect = protocol::CopyRect::read_from(&mut stream)?;
                                let src = Rect {
                                    left: copy_rect.src_x_position,
                                    top: copy_rect.src_y_position,
                                    width: rectangle.width,
                                    height: rectangle.height,
                                };
                                send!(tx_events, Event::CopyPixels { src, dst })
                            }
                            protocol::Encoding::Zrle => {
                                let length = stream.read_u32::<BigEndian>()?;
                                let mut data = Vec::with_capacity(length as usize);
                                unsafe { data.set_len(length as usize) }
                                stream.read_exact(&mut data)?;
                                debug!("<- ...compressed pixels");
                                let result =
                                    zrle_decoder.decode(format, dst, &data, |tile, pixels| {
                                        Ok(tx_events.send(Event::PutPixels(tile, pixels)).is_ok())
                                    })?;
                                if !result {
                                    break;
                                }
                            }
                            protocol::Encoding::Cursor => {
                                let mut pixels = vec![
                                    0;
                                    (rectangle.width as usize)
                                        * (rectangle.height as usize)
                                        * (format.bits_per_pixel as usize / 8)
                                ];
                                stream.read_exact(&mut pixels)?;
                                let mut mask_bits = vec![
                                    0;
                                    ((rectangle.width as usize + 7) / 8)
                                        * (rectangle.height as usize)
                                ];
                                stream.read_exact(&mut mask_bits)?;
                                send!(
                                    tx_events,
                                    Event::SetCursor {
                                        size: (rectangle.width, rectangle.height),
                                        hotspot: (rectangle.x_position, rectangle.y_position),
                                        pixels,
                                        mask_bits,
                                    }
                                )
                            }
                            protocol::Encoding::DesktopSize => {
                                send!(tx_events, Event::Resize(rectangle.width, rectangle.height))
                            }
                            _ => return Err(Error::Unexpected("encoding")),
                        };
                    }

                    send!(tx_events, Event::EndOfFrame);
                }
                protocol::S2C::Bell => send!(tx_events, Event::Bell),
                protocol::S2C::CutText(text) => send!(tx_events, Event::Clipboard(text)),
            }
        }

        Ok(())
    }
}

pub struct Client {
    stream: TcpStream,
    events: Receiver<Event>,
    name: String,
    size: (u16, u16),
    format: Arc<Mutex<protocol::PixelFormat>>,
}

impl Client {
    pub fn from_tcp_stream<Auth>(mut stream: TcpStream, shared: bool, auth: Auth) -> Result<Client>
    where
        Auth: FnOnce(&[AuthMethod]) -> Option<AuthChoice>,
    {
        let version = protocol::Version::read_from(&mut stream)?;
        debug!("<- Version::{:?}", version);
        debug!("-> Version::{:?}", version);
        protocol::Version::write_to(&version, &mut stream)?;

        let security_types = match version {
            protocol::Version::Rfb33 => {
                let security_type = protocol::SecurityType::read_from(&mut stream)?;
                debug!("<- SecurityType::{:?}", security_type);
                if security_type == protocol::SecurityType::Invalid {
                    vec![]
                } else {
                    vec![security_type]
                }
            }
            _ => {
                let security_types = protocol::SecurityTypes::read_from(&mut stream)?;
                debug!("<- {:?}", security_types);
                security_types.0
            }
        };

        if security_types.is_empty() {
            let reason = String::read_from(&mut stream)?;
            debug!("<- {:?}", reason);
            return Err(Error::Server(reason));
        }

        let mut auth_methods = Vec::new();
        for security_type in security_types {
            match security_type {
                protocol::SecurityType::None => auth_methods.push(AuthMethod::None),
                protocol::SecurityType::VncAuthentication => {
                    auth_methods.push(AuthMethod::Password)
                }
                protocol::SecurityType::AppleRemoteDesktop => {
                    auth_methods.push(AuthMethod::AppleRemoteDesktop)
                }
                _ => (),
            }
        }

        let auth_choice = auth(&auth_methods).ok_or(Error::AuthenticationUnavailable)?;

        match version {
            protocol::Version::Rfb33 => (),
            _ => {
                let used_security_type = match auth_choice {
                    AuthChoice::None => protocol::SecurityType::None,
                    AuthChoice::Password(_) => protocol::SecurityType::VncAuthentication,
                    AuthChoice::AppleRemoteDesktop(_, _) => {
                        protocol::SecurityType::AppleRemoteDesktop
                    }
                };
                debug!("-> SecurityType::{:?}", used_security_type);
                protocol::SecurityType::write_to(&used_security_type, &mut stream)?;
            }
        }

        match auth_choice {
            AuthChoice::Password(mut password) => {
                // Reverse the bits in every byte of password.
                // DES is 56-bit and as commonly implemented, it takes a 8-octet key
                // and ignores LSB of every octet; this of course would be bad for
                // ASCII passwords.
                //
                // I've spent *hours* figuring this out.
                // I hate every single fucker involved in the chain of decisions that
                // led to this authentication scheme, and doubly so because it is completely
                // undocumented in what passes for the specification of the RFB protocol.
                for i in 0..8 {
                    let c = password[i];
                    let mut cs = 0u8;
                    for j in 0..8 {
                        cs |= ((c >> j) & 1) << (7 - j)
                    }
                    password[i] = cs;
                }

                let mut challenge = [0; 16];
                stream.read_exact(&mut challenge)?;
                let response = des(&challenge, &password);
                stream.write_all(&response)?;
            }
            #[cfg(feature = "apple-auth")]
            AuthChoice::AppleRemoteDesktop(ref username, ref password) => {
                let handshake = protocol::AppleAuthHandshake::read_from(&mut stream)?;
                let response = apple_auth(username, password, &handshake);
                response.write_to(&mut stream)?;
            }
            _ => (),
        }

        let mut skip_security_result = false;
        match &(auth_choice, version) {
            &(AuthChoice::None, protocol::Version::Rfb33)
            | &(AuthChoice::None, protocol::Version::Rfb37) => skip_security_result = true,
            _ => (),
        }

        if !skip_security_result {
            match protocol::SecurityResult::read_from(&mut stream)? {
                protocol::SecurityResult::Succeeded => (),
                protocol::SecurityResult::Failed => match version {
                    protocol::Version::Rfb33 | protocol::Version::Rfb37 => {
                        return Err(Error::AuthenticationFailure(String::from("")))
                    }
                    protocol::Version::Rfb38 => {
                        let reason = String::read_from(&mut stream)?;
                        debug!("<- {:?}", reason);
                        return Err(Error::AuthenticationFailure(reason));
                    }
                },
            }
        }

        let client_init = protocol::ClientInit { shared };
        debug!("-> {:?}", client_init);
        protocol::ClientInit::write_to(&client_init, &mut stream)?;

        let server_init = protocol::ServerInit::read_from(&mut stream)?;
        debug!("<- {:?}", server_init);

        let format = Arc::new(Mutex::new(server_init.pixel_format));

        let (tx_events, rx_events) = channel();
        {
            let stream = stream.try_clone().unwrap();
            let format = format.clone();
            thread::spawn(move || {
                let mut tx_events = tx_events;
                let error = Event::pump(stream, format, &mut tx_events).err();
                let _ = tx_events.send(Event::Disconnected(error));
            });
        }

        Ok(Client {
            stream,
            events: rx_events,
            name: server_init.name,
            size: (
                server_init.framebuffer_width,
                server_init.framebuffer_height,
            ),
            format,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn size(&self) -> (u16, u16) {
        self.size
    }
    pub fn format(&self) -> protocol::PixelFormat {
        *self.format.lock().unwrap()
    }

    pub fn set_encodings(&mut self, encodings: &[protocol::Encoding]) -> Result<()> {
        let set_encodings = protocol::C2S::SetEncodings(Vec::from(encodings));
        debug!("-> {:?}", set_encodings);
        protocol::C2S::write_to(&set_encodings, &mut self.stream)?;
        Ok(())
    }

    pub fn request_update(&mut self, rect: Rect, incremental: bool) -> Result<()> {
        let update_req = protocol::C2S::FramebufferUpdateRequest {
            incremental,
            x_position: rect.left,
            y_position: rect.top,
            width: rect.width,
            height: rect.height,
        };
        trace!("-> {:?}", update_req);
        protocol::C2S::write_to(&update_req, &mut self.stream)?;
        Ok(())
    }

    pub fn send_key_event(&mut self, down: bool, key: u32) -> Result<()> {
        let key_event = protocol::C2S::KeyEvent { down, key };
        debug!("-> {:?}", key_event);
        protocol::C2S::write_to(&key_event, &mut self.stream)?;
        Ok(())
    }

    pub fn send_pointer_event(&mut self, buttons: u8, x: u16, y: u16) -> Result<()> {
        let pointer_event = protocol::C2S::PointerEvent {
            button_mask: buttons,
            x_position: x,
            y_position: y,
        };
        debug!("-> {:?}", pointer_event);
        protocol::C2S::write_to(&pointer_event, &mut self.stream)?;
        Ok(())
    }

    pub fn update_clipboard(&mut self, text: &str) -> Result<()> {
        let cut_text = protocol::C2S::CutText(String::from(text));
        debug!("-> {:?}", cut_text);
        protocol::C2S::write_to(&cut_text, &mut self.stream)?;
        Ok(())
    }

    // Note that due to inherent weaknesses of the VNC protocol, this
    // function is prone to race conditions that break the connection framing.
    // The ZRLE encoding is self-delimiting and if both the client and server
    // support and use it, there can be no race condition, but we currently don't.
    pub fn set_format(&mut self, format: protocol::PixelFormat) -> Result<()> {
        debug!("0");
        // Request (and discard) one full update to try and ensure that there
        // are no FramebufferUpdate's in the buffers somewhere.
        // This is not fully robust though (and cannot possibly be).
        let _ = self.poll_iter().count(); // drain it
        let framebuffer_rect = Rect {
            left: 0,
            top: 0,
            width: self.size.0,
            height: self.size.1,
        };
        debug!("01");
        self.request_update(framebuffer_rect, false)?;
        debug!("02");
        'outer: loop {
            for event in self.poll_iter() {
                debug!("got an event");
                match event {
                    Event::PutPixels(rect, _) if rect == framebuffer_rect => break 'outer,
                    Event::EndOfFrame => break 'outer,
                    //_ => {debug!("Event: {:?}", event)},
                    _ => ()
                }
            }
        }

        debug!("1");
        // Since VNC is fully client-driven, by this point the event thread is stuck
        // waiting for the next message and the server is not sending us anything,
        // so it's safe to switch to the new pixel format.
        let set_pixel_format = protocol::C2S::SetPixelFormat(format);
        debug!("-> {:?}", set_pixel_format);
        protocol::C2S::write_to(&set_pixel_format, &mut self.stream)?;
        debug!("2");
        *self.format.lock().unwrap() = format;

        debug!("3");
        Ok(())
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        match self.events.try_recv() {
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
            Ok(Event::Resize(width, height)) => {
                self.size = (width, height);
                Some(Event::Resize(width, height))
            }
            Ok(event) => Some(event),
        }
    }

    pub fn poll_iter(&mut self) -> EventPollIterator {
        EventPollIterator { client: self }
    }

    pub fn disconnect(self) -> Result<()> {
        self.stream.shutdown(Shutdown::Both)?;
        Ok(())
    }
}

pub struct EventPollIterator<'a> {
    client: &'a mut Client,
}

impl<'a> Iterator for EventPollIterator<'a> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        self.client.poll_event()
    }
}
