#[macro_use]
extern crate log;
extern crate byteorder;
extern crate flate2;

mod device;
mod framebuffer;
#[macro_use]
mod geom;
mod color;
mod input;
mod security;
mod settings;
mod vnc;

pub use crate::framebuffer::image::ReadonlyPixmap;
use crate::framebuffer::{Framebuffer, KoboFramebuffer1, KoboFramebuffer2, Pixmap, UpdateMode};
use crate::geom::Rectangle;
use crate::vnc::{client, Client, Encoding, Rect};
use clap::{value_t, App, Arg};
use log::{debug, error, info};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use vnc::PixelFormat;

use anyhow::{Context as ResultExt, Error};

use crate::device::CURRENT_DEVICE;

const FB_DEVICE: &str = "/dev/fb0";

#[repr(align(256))]
pub struct PostProcBin {
    data: [u8; 256],
}

fn main() -> Result<(), Error> {
    env_logger::init();

    let matches = App::new("einkvnc")
        .about("VNC client")
        .arg(
            Arg::with_name("HOST")
                .help("server hostname or IP")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::with_name("PORT")
                .help("server port (default: 5900)")
                .index(2),
        )
        .arg(
            Arg::with_name("USERNAME")
                .help("server username")
                .long("username")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("PASSWORD")
                .help("server password")
                .long("password")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("EXCLUSIVE")
                .help("request a non-shared session")
                .long("exclusive"),
        )
        .arg(
            Arg::with_name("CONTRAST")
                .help("apply a post processing contrast filter")
                .long("contrast")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("GRAYPOINT")
                .help("the gray point of the post processing contrast filter")
                .long("graypoint")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("WHITECUTOFF")
                .help("apply a post processing filter to turn colors greater than the specified value to white (255)")
                .long("whitecutoff")
                .takes_value(true),
        ).arg(
            Arg::with_name("ROTATE")
                .help("rotation (1-4), tested on a Clara HD, try at own risk")
                .long("rotate")
                .takes_value(true),
        ) 
        .get_matches();

    let host = matches.value_of("HOST").unwrap();
    let port = value_t!(matches.value_of("PORT"), u16).unwrap_or(5900);
    let username = matches.value_of("USERNAME");
    let password = matches.value_of("PASSWORD");
    let contrast_exp = value_t!(matches.value_of("CONTRAST"), f32).unwrap_or(1.0);
    let contrast_gray_point = value_t!(matches.value_of("GRAYPOINT"), f32).unwrap_or(224.0);
    let white_cutoff = value_t!(matches.value_of("WHITECUTOFF"), u8).unwrap_or(255);
    let exclusive = matches.is_present("EXCLUSIVE");
    let rotate = value_t!(matches.value_of("ROTATE"), i8).unwrap_or(1);

    info!("connecting to {}:{}", host, port);
    let stream = match std::net::TcpStream::connect((host, port)) {
        Ok(stream) => stream,
        Err(error) => {
            error!("cannot connect to {}:{}: {}", host, port, error);
            std::process::exit(1)
        }
    };

    let mut vnc = match Client::from_tcp_stream(stream, !exclusive, |methods| {
        debug!("available authentication methods: {:?}", methods);
        for method in methods {
            match method {
                client::AuthMethod::None => return Some(client::AuthChoice::None),
                client::AuthMethod::Password => {
                    return match password {
                        None => None,
                        Some(ref password) => {
                            let mut key = [0; 8];
                            for (i, byte) in password.bytes().enumerate() {
                                if i == 8 {
                                    break;
                                }
                                key[i] = byte
                            }
                            Some(client::AuthChoice::Password(key))
                        }
                    }
                }
                client::AuthMethod::AppleRemoteDesktop => match (username, password) {
                    (Some(username), Some(password)) => {
                        return Some(client::AuthChoice::AppleRemoteDesktop(
                            username.to_owned(),
                            password.to_owned(),
                        ))
                    }
                    _ => (),
                },
            }
        }
        None
    }) {
        Ok(vnc) => vnc,
        Err(error) => {
            error!("cannot initialize VNC session: {}", error);
            std::process::exit(1)
        }
    };

    let (width, height) = vnc.size();
    info!(
        "connected to \"{}\", {}x{} framebuffer",
        vnc.name(),
        width,
        height
    );

    let vnc_format = vnc.format();
    info!("received {:?}", vnc_format);

    vnc.set_encodings(&[Encoding::CopyRect, Encoding::Zrle])
        .unwrap();

    vnc.request_update(
        Rect {
            left: 0,
            top: 0,
            width,
            height,
        },
        false,
    )
    .unwrap();

    #[cfg(feature = "eink_device")]
    debug!(
        "running on device model=\"{}\" /dpi={} /dims={}x{}", 
        CURRENT_DEVICE.model,
        CURRENT_DEVICE.dpi,
        CURRENT_DEVICE.dims.0,
        CURRENT_DEVICE.dims.1
    );

    let mut fb: Box<dyn Framebuffer> = if CURRENT_DEVICE.mark() != 8 {
        Box::new(
            KoboFramebuffer1::new(FB_DEVICE)
                .context("can't create framebuffer")
                .unwrap(),
        )
    } else {
        Box::new(
            KoboFramebuffer2::new(FB_DEVICE)
                .context("can't create framebuffer")
                .unwrap(),
        )
    };

    #[cfg(feature = "eink_device")]
    {
        let startup_rotation = rotate;
        fb.set_rotation(startup_rotation).ok();
    }

    let post_proc_bin = PostProcBin {
        data: (0..=255)
            .map(|i| {
                if contrast_exp == 1.0 {
                    i
                } else {
                    let gray = contrast_gray_point;

                    let rem_gray = 255.0 - gray;
                    let inv_exponent = 1.0 / contrast_exp;

                    let raw_color = i as f32;
                    if raw_color < gray {
                        (gray * (raw_color / gray).powf(contrast_exp)) as u8
                    } else if raw_color > gray {
                        (gray + rem_gray * ((raw_color - gray) / rem_gray).powf(inv_exponent)) as u8
                    } else {
                        gray as u8
                    }
                }
            })
            .map(|i| -> u8 {
                if i > white_cutoff {
                    255
                } else {
                    i
                }
            })
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap(),
    };

    const FRAME_MS: u64 = 1000 / 30;

    const MAX_DIRTY_REFRESHES: usize = 500;

    let mut dirty_rects: Vec<Rectangle> = Vec::new();
    let mut dirty_rects_since_refresh: Vec<Rectangle> = Vec::new();
    let mut has_drawn_once = false;
    let mut dirty_update_count = 0;

    let mut time_at_last_draw = Instant::now();

    let fb_rect = rect![0, 0, width as i32, height as i32];

    let post_proc_enabled = contrast_exp != 1.0;

    'running: loop {
        let time_at_sol = Instant::now();

        for event in vnc.poll_iter() {
            use client::Event;

            match event {
                Event::Disconnected(None) => break 'running,
                Event::Disconnected(Some(error)) => {
                    error!("server disconnected: {:?}", error);
                    break 'running;
                }
                Event::PutPixels(vnc_rect, ref pixels) => {
                    debug!("Put pixels");

                    let elapsed_ms = time_at_sol.elapsed().as_millis();
                    debug!("network Δt: {}", elapsed_ms);

                    let scale_down = 
                        pixels
                            .iter()
                            .step_by(4)
                            .map(|&c| post_proc_bin.data[c as usize])
                            .collect();

                    let post_proc_pixels = if post_proc_enabled {
                        pixels
                            .iter()
                            .step_by(4)
                            .map(|&c| post_proc_bin.data[c as usize])
                            .collect()
                    } else {
                        Vec::new()
                    };

                    let pixels = if post_proc_enabled {
                        &post_proc_pixels
                    } else {
                        &scale_down
                    };

                    let w = vnc_rect.width as u32;
                    let h = vnc_rect.height as u32;
                    let l = vnc_rect.left as u32;
                    let t = vnc_rect.top as u32;

                    let pixmap = ReadonlyPixmap {
                        width: w as u32,
                        height: h as u32,
                        data: pixels,
                    };
                    debug!("Put pixels {} {} {} size {}",w,h,w*h,pixels.len());

                    let elapsed_ms = time_at_sol.elapsed().as_millis();
                    debug!("postproc Δt: {}", elapsed_ms);

                    #[cfg(feature = "eink_device")]
                    {
                        for y in 0..pixmap.height {
                            for x in 0..pixmap.width {
                                let px = x + l;
                                let py = y + t;
                                let color = pixmap.get_pixel(x, y);
                                fb.set_pixel(px, py, color);
                            }
                        }
                    }

                    let elapsed_ms = time_at_sol.elapsed().as_millis();
                    debug!("draw Δt: {}", elapsed_ms);

                    let w = vnc_rect.width as i32;
                    let h = vnc_rect.height as i32;
                    let l = vnc_rect.left as i32;
                    let t = vnc_rect.top as i32;

                    let delta_rect = rect![l, t, l + w, t + h];
                    if delta_rect == fb_rect {
                        dirty_rects.clear();
                        dirty_rects_since_refresh.clear();
                        #[cfg(feature = "eink_device")]
                        {
                            if !has_drawn_once || dirty_update_count > MAX_DIRTY_REFRESHES {
                                fb.update(&fb_rect, UpdateMode::Full).ok();
                                dirty_update_count = 0;
                                has_drawn_once = true;
                            } else {
                                fb.update(&fb_rect, UpdateMode::Partial).ok();
                            }
                        }
                    } else {
                        push_to_dirty_rect_list(&mut dirty_rects, delta_rect);
                    }

                    let elapsed_ms = time_at_sol.elapsed().as_millis();
                    debug!("rects Δt: {}", elapsed_ms);
                }
                Event::CopyPixels { src, dst } => {
                    debug!("Copy pixels!");

                    #[cfg(feature = "eink_device")]
                    {
                        let src_left = src.left as u32;
                        let src_top = src.top as u32;

                        let dst_left = dst.left as u32;
                        let dst_top = dst.top as u32;

                        let mut intermediary_pixmap =
                            Pixmap::new(dst.width as u32, dst.height as u32);

                        for y in 0..intermediary_pixmap.height {
                            for x in 0..intermediary_pixmap.width {
                                let color = fb.get_pixel(src_left + x, src_top + y);
                                intermediary_pixmap.set_pixel(x, y, color);
                            }
                        }

                        for y in 0..intermediary_pixmap.height {
                            for x in 0..intermediary_pixmap.width {
                                let color = intermediary_pixmap.get_pixel(x, y);
                                fb.set_pixel(dst_left + x, dst_top + y, color);
                            }
                        }
                    }

                    let delta_rect = rect![
                        dst.left as i32,
                        dst.top as i32,
                        (dst.left + dst.width) as i32,
                        (dst.top + dst.height) as i32
                    ];
                    push_to_dirty_rect_list(&mut dirty_rects, delta_rect);
                }
                Event::EndOfFrame => {
                    debug!("End of frame!");

                    if !has_drawn_once {
                        has_drawn_once = dirty_rects.len() > 0;
                    }

                    dirty_update_count += 1;

                    if dirty_update_count > MAX_DIRTY_REFRESHES {
                        info!("Full refresh!");
                        for dr in &dirty_rects_since_refresh {
                            #[cfg(feature = "eink_device")]
                            {
                                fb.update(&dr, UpdateMode::Full).ok();
                            }
                        }
                        dirty_update_count = 0;
                        dirty_rects_since_refresh.clear();
                    } else {
                        for dr in &dirty_rects {
                            debug!("Updating dirty rect {:?}", dr);

                            #[cfg(feature = "eink_device")]
                            {
                                if dr.height() < 100 && dr.width() < 100 {
                                    debug!("Fast mono update!");
                                    fb.update(&dr, UpdateMode::FastMono).ok();
                                } else {
                                    fb.update(&dr, UpdateMode::Partial).ok();
                                }
                            }

                            push_to_dirty_rect_list(&mut dirty_rects_since_refresh, *dr);
                        }

                        time_at_last_draw = Instant::now();
                    }

                    dirty_rects.clear();
                }
                // x => info!("{:?}", x), /* ignore unsupported events */
                _ => (),
            }
        }

        if FRAME_MS > time_at_sol.elapsed().as_millis() as u64 {
            if dirty_rects_since_refresh.len() > 0 && time_at_last_draw.elapsed().as_secs() > 3 {
                for dr in &dirty_rects_since_refresh {
                    #[cfg(feature = "eink_device")]
                    {
                        fb.update(&dr, UpdateMode::Full).ok();
                    }
                }
                dirty_update_count = 0;
                dirty_rects_since_refresh.clear();
            }

            if FRAME_MS > time_at_sol.elapsed().as_millis() as u64 {
                thread::sleep(Duration::from_millis(
                    FRAME_MS - time_at_sol.elapsed().as_millis() as u64,
                ));
            }
        } else {
            info!(
                "Missed frame, excess Δt: {}ms",
                time_at_sol.elapsed().as_millis() as u64 - FRAME_MS
            );
        }

        vnc.request_update(
            Rect {
                left: 0,
                top: 0,
                width,
                height,
            },
            true,
        )
        .unwrap();
    }

    Ok(())
}

fn push_to_dirty_rect_list(list: &mut Vec<Rectangle>, rect: Rectangle) {
    for dr in list.iter_mut() {
        if dr.contains(&rect) {
            return;
        }
        if rect.contains(&dr) {
            *dr = rect;
            return;
        }
        if rect.extends(&dr) {
            dr.absorb(&rect);
            return;
        }
    }

    list.push(rect);
}
