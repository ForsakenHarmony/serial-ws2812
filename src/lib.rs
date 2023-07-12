use std::{
	io,
	io::{Read, Write},
};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serial_ws2812_shared::{DEVICE_ERROR_MESSAGE, DEVICE_INIT_MESSAGE, DEVICE_MESSAGE_TYPE_LEN, DEVICE_OK_MESSAGE, DEVICE_PARTIAL_MESSAGE, DEVICE_PRODUCT_NAME, SET_LEDS_MESSAGE, SET_STRIPS_MESSAGE, UPDATE_MESSAGE};
use serialport::{SerialPort, SerialPortType};

pub struct Config {
	pub strips: usize,
	pub leds: usize,
}

pub struct SerialWs2812 {
	config: Config,
	port: Box<dyn SerialPort>,

	initialized: bool,
}

impl SerialWs2812 {
	pub fn new(serial_device: String, config: Config) -> Result<Self> {
		let baud_rate = 921_600;

		let builder = serialport::new(&serial_device, baud_rate).timeout(Duration::from_millis(10));
		let port = builder.open().context(format!("opening device \"{}\"", serial_device))?;

		Ok(Self {
			config,
			port,

			initialized: false,
		})
	}

	pub fn find(config: Config) -> Result<Self> {
		let ports = serialport::available_ports().expect("No ports found!");
		let mut serial_device = None;

		for p in ports {
			if let SerialPortType::UsbPort(usb) = p.port_type {
				if usb.product == Some(DEVICE_PRODUCT_NAME.to_string()) {
					serial_device = Some(p.port_name);
				}
			}
		}

		let Some(serial_device) = serial_device else {
			bail!("no serial to ws2812 device found");
		};

		Self::new(serial_device, config)
	}

	fn reset_to_command(&mut self) -> Result<()> {
		let mut buffer = [0u8; DEVICE_MESSAGE_TYPE_LEN * 4];

		let mut has_printed = 0;
		let mut counter = 0;

		println!("trying to reset device to start of command");

		loop {
			let res = self.port.read(&mut buffer);
			let read_bytes = match res {
				Ok(n) => n,
				Err(e) if e.kind() == io::ErrorKind::TimedOut => {
					if has_printed == 0 {
						println!("read timeout, writing null bytes to force a response");
						has_printed += 1;
					}

					counter += 1;
					if counter < 8 {
						self.port.write_all(&[0u8])?;
					} else {
						self.port.write_all(&[0u8; 32])?;
					}

					continue;
				}
				Err(e) => bail!(e),
			};

			// println!("received: {:?}", String::from_utf8_lossy(&buffer[..read_bytes]));

			// if we receive more than one byte we're probably in the branch that writes 32 bytes and need to repeat the process
			if read_bytes > 1 {
				counter = 0;
				continue;
			}

			if &buffer[..1] == DEVICE_INIT_MESSAGE || &buffer[..1] == DEVICE_ERROR_MESSAGE {
				break;
			}
		}

		println!("reset successful");

		Ok(())
	}

	pub fn configure(&mut self) -> Result<()> {
		if !self.initialized {
			self.reset_to_command()?;
			self.initialized = true;
		}

		self.send_command( SET_STRIPS_MESSAGE, &u32::to_le_bytes(self.config.strips as u32))?;
		self.send_command( SET_LEDS_MESSAGE, &u32::to_le_bytes(self.config.leds as u32))?;

		Ok(())
	}

	pub fn send_leds(&mut self, leds: &[u8]) -> Result<(Duration, Duration)> {
		if !self.initialized {
			self.configure()?;
		}

		self.send_command(UPDATE_MESSAGE, leds)
	}

	fn send_command(&mut self, command: &[u8], data: &[u8]) -> Result<(Duration, Duration)> {
		let mut output = [0u8; DEVICE_MESSAGE_TYPE_LEN];

		let command_start = Instant::now();

		if self.serial_write(command)? != command.len() {
			bail!("command message failed to write");
		}
		if self.port.read(&mut output)? != 1 {
			bail!("no response to command");
		}
		if &output != DEVICE_PARTIAL_MESSAGE {
			bail!("unexpected response to command: {:?} (expected \"p\")", output)
		}

		let command_duration = Instant::now() - command_start;

		let data_start = Instant::now();

		if self.serial_write(data)? != data.len() {
			bail!("command data message failed to write");
		}
		if self.port.read(&mut output)? != 1 {
			bail!("no response to command data");
		}
		if &output != DEVICE_OK_MESSAGE {
			bail!("unexpected response to command data: {:?} (expected \"k\")", output)
		}

		let data_duration = Instant::now() - data_start;

		Ok((command_duration, data_duration))
	}

	fn serial_write(&mut self, buffer: &[u8]) -> Result<usize> {
		match self.port.write_all(&buffer) {
			Ok(_) => {
				Ok(buffer.len())
			}
			Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
				println!("WARNING: serial timeout");
				Ok(0)
			}
			Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
				println!("WARNING: serial interrupted");
				Ok(0)
			}
			Err(e) => {
				bail!(e);
			}
		}
	}
}
