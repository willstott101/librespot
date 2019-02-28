use super::{Open, Sink};
extern crate rodio;
use std::time::Duration;
use std::io;
use std::process::exit;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

pub struct RodioSink {
    rodio_sink: rodio::Sink,
    send: Option<SyncSender<i16>>,
}

fn list_outputs() {
    println!("Default Audio Device:\n  {:?}", rodio::default_output_device().map(|e| e.name()));

    println!("Available Audio Devices:");
    for device in rodio::output_devices() {
        println!("- {}", device.name());
        // Output formats
        if let Ok(fmt) = device.default_output_format() {
            println!("  Default format:\n    {:?}", fmt);
        }
        let mut output_formats = match device.supported_output_formats() {
            Ok(f) => f.peekable(),
            Err(e) => {
                println!("Error: {:?}", e);
                continue;
            },
        };
        if output_formats.peek().is_some() {
            println!("  All formats:");
            for format in output_formats {
                println!("    {:?}", format);
            }
        }
    }
}

impl Open for RodioSink {
    fn open(device: Option<String>) -> RodioSink {
        info!("Using rodio sink");

        let mut rodio_device = rodio::default_output_device().expect("no output device available");
        if device.is_some() {
            let device_name = device.unwrap();

            if device_name == "?".to_string() {
                list_outputs();
                exit(0)
            }
            let mut found = false;
            for d in rodio::output_devices() {
                if d.name() == device_name {
                    rodio_device = d;
                    found = true;
                    break;
                }
            }
            if !found {
                println!("No output sink matching '{}' found.", device_name);
                exit(0)
            }
        }

        let sink = rodio::Sink::new(&rodio_device);
        let source = RodioSink {
            rodio_sink: sink,
            send: None,
        };

        source
    }
}

impl Sink for RodioSink {
    fn start(&mut self) -> io::Result<()> {
        //                  100ms = 2 * 4410
        let (tx, rx) = sync_channel(2 * 4096);
        self.send = Some(tx);
        let source = LibrespotSource {
            recv: rx,
        };
        self.rodio_sink.append(source);
        self.rodio_sink.play();
        Ok(())
    }

    fn stop(&mut self) -> io::Result<()> {
        self.send = None;
        Ok(())
    }

    fn write(&mut self, data: &[i16]) -> io::Result<()> {
        match self.send {
            Some(ref sender) => {
                for s in data.iter() {
                    let r = sender.send(*s);
                    if r.is_err() {
                        return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Rodio Sink: Reciever disconnected."));
                    } else {
                        r.unwrap();
                    }
                }
            },
            None => (),
        }
        Ok(())
    }
}


struct LibrespotSource {
    recv: Receiver<i16>,
}

impl Iterator for LibrespotSource {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        let mut queue_iter = self.recv.try_iter();
        queue_iter.next()
    }
}

impl rodio::Source for LibrespotSource {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    #[inline]
    fn channels(&self) -> u16 {
        2
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        44100
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}