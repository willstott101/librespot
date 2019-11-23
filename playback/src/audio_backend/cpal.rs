use super::{Open, Sink};
extern crate sample;
extern crate cpal;
use std::io;
use std::process::exit;
use std::sync::mpsc::{sync_channel, SyncSender};
use audio_backend::cpal::cpal::traits::{DeviceTrait, StreamTrait, HostTrait};

use self::sample::{interpolate, signal, Sample, Signal};

struct ResampleParams {
    last_frame: [i16; 2],
    target_sample_rate: f64,
}

#[allow(dead_code)]
struct PlaybackParams {
    stream: cpal::Stream, // Only kept as Drop is implemented.
    send: SyncSender<i16>,
    resample: Option<ResampleParams>,
}

pub struct CpalSink {
    device_name: Option<String>,
    playback: Option<PlaybackParams>,
}

fn list_formats(ref device: &cpal::Device) {
    let default_fmt = match device.default_output_format() {
        Ok(fmt) => cpal::SupportedFormat::from(fmt),
        Err(e) => {
            warn!("Error getting default cpal::Device format: {:?}", e);
            return;
        },
    };

    let mut output_formats = match device.supported_output_formats() {
        Ok(f) => f.peekable(),
        Err(e) => {
            warn!("Error getting supported cpal::Device formats: {:?}", e);
            return;
        },
    };

    if output_formats.peek().is_some() {
        debug!("  Available formats:");
        for format in output_formats {
            let s = format!("{}ch, {:?}, min {:?}, max {:?}", format.channels, format.data_type, format.min_sample_rate, format.max_sample_rate);
            if format == default_fmt {
                debug!("    (default) {}", s);
            } else {
                debug!("    {:?}", format);
            }
        }
    }
}

fn get_name(ref device: &cpal::Device) -> String {
    device.name().unwrap_or("NO_NAME".to_string())
}

fn list_outputs() {
    let host = cpal::default_host();
    let default_device = host.default_output_device().unwrap();
    let default_device_name = get_name(&default_device);
    println!("Default Audio Device:\n  {}", default_device_name);
    list_formats(&default_device);

    println!("Other Available Audio Devices:");
    for device in host.output_devices().unwrap() {
        let name = get_name(&device);
        if name != default_device_name {
            println!("  {}", name);
            list_formats(&device);
        }
    }
}

fn match_output(device_name: &Option<String>) -> cpal::Device {
    let host = cpal::default_host();
    match device_name {
        Some(dn) => {
            let mut cpal_device = None;
            for device in host.output_devices().unwrap() {
                if get_name(&device) == *dn {
                    cpal_device = Some(device);
                    break;
                }
            }
            match cpal_device {
                Some(cd) => cd,
                None => {
                    println!("No output sink matching '{}' found.", dn);
                    exit(0)
                }
            }
        },
        None => host.default_output_device().expect("no output device available")
    }
}

impl Open for CpalSink {
    fn open(device_name: Option<String>) -> CpalSink {
        debug!("Using CPAL sink");

        if device_name == Some("?".to_string()) {
            list_outputs();
            exit(0)
        }

        CpalSink {
            device_name,
            playback: None
        }
    }
}

impl Sink for CpalSink {
    fn start(&mut self) -> io::Result<()> {
        let device = match_output(&self.device_name);
        let format = device.default_output_format().unwrap();

        // buffer for samples from librespot (~10ms)
        let (send, rx) = sync_channel::<i16>(2 * 1024 * 4);

        let stream = device.build_output_stream(&format, move |data| {
            match data {
                cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::I16(mut buffer) } => {
                    for sample in buffer.iter_mut() {
                        let recv = rx.try_recv().unwrap_or(0);
                        *sample = recv;
                    }
                },
                cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::U16(mut buffer) } => {
                    for sample in buffer.iter_mut() {
                        let recv = rx.try_recv().unwrap_or(0);
                        *sample = recv.to_sample::<u16>();
                    }
                },
                cpal::StreamData::Output { buffer: cpal::UnknownTypeOutputBuffer::F32(mut buffer) } => {
                    for sample in buffer.iter_mut() {
                        let recv = rx.try_recv().unwrap_or(0);
                        *sample = recv.to_sample::<f32>();
                    }
                },
                _ => (),
            }
        }, move |err| {
            eprintln!("an error occurred on stream: {}", err);
        }).unwrap();

        stream.play().expect("Could not start playback.");

        self.playback = Some(PlaybackParams {
            stream,
            send,
            resample: match format.sample_rate.0 {
                44100 => None,
                sample_rate => {
                    debug!("Resampling from 44100 to {:?}", sample_rate);
                    Some(ResampleParams{
                        last_frame: [0, 0],
                        target_sample_rate: sample_rate as f64
                    })
                },
            }
        });
        Ok(())
    }

    fn stop(&mut self) -> io::Result<()> {
        match self.playback {
            Some(_) => {
                self.playback = None;
            },
            None => (),
        }
        Ok(())
    }

    fn write(&mut self, data: &[i16]) -> io::Result<()> {
        match &mut self.playback {
            Some(pb) => {
                match pb.resample {
                    None => {
                        for s in data {
                            let res = pb.send.send(*s);
                            if res.is_err() {
                                error!("cpal: cannot write to channel");
                            }
                        }
                    },
                    Some(ref mut params) => {
                        // Copy the decoded audio into a Signal.
                        let signal = signal::from_interleaved_samples_iter::<_, [i16; 2]>(data.iter().map(|v| *v));
                        // Instantiate a Linear interpolator using frame values from the last chunk.
                        let interpolator = interpolate::Linear::new(params.last_frame.clone(),
                                                                    params.last_frame.clone());
                        // Interpolate into a new Signal object.
                        let new_signal = signal.from_hz_to_hz(interpolator, 44100 as f64, params.target_sample_rate);

                        // Send to the the reciever.
                        for frame in new_signal.until_exhausted() {
                            let res = pb.send.send(frame[0]);
                            if res.is_err() {
                                error!("cpal: cannot write to channel");
                            }
                            let res = pb.send.send(frame[1]);
                            if res.is_err() {
                                error!("cpal: cannot write to channel");
                            }
                            // Store final frame for seamless interpolation into the next chunk.
                            params.last_frame = frame;
                        }
                    },
                };
            },
            None => (),
        }
        Ok(())
    }
}
