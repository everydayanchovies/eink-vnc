# eInk VNC

A lightweight CLI (command line interface) tool to view a remote screen over VNC, designed to work on eInk screens.
For now, you can only view, so you'll have to connect a keyboard to the serving computer, or find some other way to interact with it.

This tool has been confirmed to work on the Kobo Libra 2, and should work on all Kobo devices.
It was optimized for text based workflows (document reading and writing), doing that it achieves a framerate of 30 fps.

**It has only been confirmed to work with TightVNC as the server.**
Due to the unusual pixel format.

## Warning

The screen can refresh up to 30 times per second, this will degrade the eInk display rapidly.
Do not use with fast changing content like videos.

Furthermore, this tool was only tested on a single device (Kobo Libra 2).
**It is possible that it will damage yours.**
*I cannot be held responsible, use this tool at your own risk.*

[einkvnc_demo_kobo_rotated.webm](https://user-images.githubusercontent.com/4356678/184497681-683af36b-e226-47fc-8993-34a5b356edba.webm)

## Usage

You can use this tool by connecting to the eInk device through SSH, or using menu launchers on the device itself.

To connect to a VNC server:

``` shell
./einkvnc [IP_ADDRESS] [PORT] [OPTIONS]
```

For example:

``` shell
./einkvnc 192.168.2.1 5902 --password abcdefg123 --contrast 2 
```

For faster framerates, use USB networking (see https://www.mobileread.com/forums/showthread.php?t=254214).

## Derivatives

The code responsible for rendering to the eInk display is written by baskerville and taken from https://github.com/baskerville/plato.
The code responsible for communicating using the VNC protocol is written by whitequark and taken from https://github.com/whitequark/rust-vnc.

Thank you both.
