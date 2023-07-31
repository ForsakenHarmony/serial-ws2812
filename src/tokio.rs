#[cfg(feature = "timings")]
use std::time::Instant;
use std::{io, time::Duration};

use serial_ws2812_shared::{
	DEVICE_ERROR_MESSAGE,
	DEVICE_INIT_MESSAGE,
	DEVICE_MESSAGE_TYPE_LEN,
	DEVICE_OK_MESSAGE,
	DEVICE_PARTIAL_MESSAGE,
	DEVICE_PRODUCT_NAME,
	SET_LEDS_MESSAGE,
	SET_STRIPS_MESSAGE,
	UPDATE_MESSAGE,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialPortType, SerialStream};
use tracing::info;

use crate::{Config, Error, Result, WriteResult};

pub struct SerialWs2812 {
	config: Config,
	port:   SerialStream,

	initialized: bool,
}

impl SerialWs2812 {
	/// Create a new instance with the given serial device and config.
	pub fn new(serial_device: String, config: Config) -> Result<Self> {
		let baud_rate = 921_600;

		let builder =
			tokio_serial::new(serial_device, baud_rate).timeout(Duration::from_millis(50));
		let port = builder.open_native_async()?;

		Ok(Self {
			config,
			port,

			initialized: false,
		})
	}

	/// Finds the first available serial device with product name "Serial WS2812" and creates a new instance of this controller struct from it.
	///
	/// If more than one device is connected the returned device will be the first the OS lists.
	pub fn find(config: Config) -> Result<Option<Self>> {
		let ports = tokio_serial::available_ports()?;
		let mut serial_device = None;

		for p in ports {
			if let SerialPortType::UsbPort(usb) = p.port_type {
				if usb.product == Some(DEVICE_PRODUCT_NAME.to_string())
					|| usb.product == Some(DEVICE_PRODUCT_NAME.replace(' ', "_"))
				{
					serial_device = Some(p.port_name);
				}
			}
		}

		let Some(serial_device) = serial_device else {
			return Ok(None);
		};

		Ok(Some(Self::new(serial_device, config)?))
	}

	async fn reset_to_command(&mut self) -> Result<()> {
		let mut buffer = [0u8; DEVICE_MESSAGE_TYPE_LEN * 4];

		let mut has_printed = 0;
		let mut counter = 0;

		info!("trying to reset device to start of command");
		self.port.set_timeout(Duration::from_millis(10))?;

		loop {
			let res = self.port.read(&mut buffer).await;
			let read_bytes = match res {
				Ok(n) => n,
				Err(e) if e.kind() == io::ErrorKind::TimedOut => {
					if has_printed == 0 {
						info!("read timeout, writing null bytes to force a response");
						has_printed += 1;
					}

					counter += 1;
					if counter < 8 {
						self.port.write_all(&[0u8]).await?;
					} else {
						self.port.write_all(&[0u8; 32]).await?;
					}

					continue;
				}
				Err(e) => return Err(e.into()),
			};

			// if we receive more than one byte we're probably in the branch that writes 32 bytes and need to repeat the process
			if read_bytes > 1 {
				counter = 0;
				continue;
			}

			if &buffer[..1] == DEVICE_INIT_MESSAGE || &buffer[..1] == DEVICE_ERROR_MESSAGE {
				break;
			}
		}

		self.port.set_timeout(Duration::from_millis(50))?;
		info!("reset successful");

		Ok(())
	}

	/// Sets the configuration for the instance.
	pub async fn set_config(&mut self, config: Config) -> Result<()> {
		self.config = config;
		self.configure().await
	}

	pub async fn configure(&mut self) -> Result<()> {
		if !self.initialized {
			self.reset_to_command().await?;
			self.initialized = true;
		}

		self.send_command(
			SET_STRIPS_MESSAGE,
			&u32::to_le_bytes(self.config.strips as u32),
		)
		.await?;
		self.send_command(SET_LEDS_MESSAGE, &u32::to_le_bytes(self.config.leds as u32))
			.await?;

		Ok(())
	}

	/// Send all bytes to the microcontroller, the length must be the configured amount of leds * strips * 3.
	pub async fn send_leds(&mut self, leds: &[u8]) -> Result<WriteResult> {
		if !self.initialized {
			self.configure().await?;
		}

		self.send_command(UPDATE_MESSAGE, leds).await
	}

	async fn send_command(&mut self, command: &[u8], data: &[u8]) -> Result<WriteResult> {
		let mut output = [0u8; DEVICE_MESSAGE_TYPE_LEN];

		#[cfg(feature = "timings")]
		let command_start = Instant::now();

		if self.serial_write(command).await? != command.len() {
			return Err(Error::IncompleteWrite);
		}
		if self.port.read(&mut output).await? != 1 {
			return Err(Error::NoResponse);
		}
		if &output != DEVICE_PARTIAL_MESSAGE {
			return Err(Error::UnexpectedResponse {
				expected: String::from_utf8_lossy(DEVICE_PARTIAL_MESSAGE).to_string(),
				received: format!("{:?}", output),
			});
		}

		#[cfg(feature = "timings")]
		let data_start = Instant::now();

		if self.serial_write(data).await? != data.len() {
			return Err(Error::IncompleteWrite);
		}
		if self.port.read(&mut output).await? != 1 {
			return Err(Error::NoResponse);
		}
		if &output != DEVICE_OK_MESSAGE {
			return Err(Error::UnexpectedResponse {
				expected: String::from_utf8_lossy(DEVICE_OK_MESSAGE).to_string(),
				received: format!("{:?}", output),
			});
		}

		#[cfg(feature = "timings")]
		let end = Instant::now();

		#[cfg(feature = "timings")]
		return Ok((data_start - command_start, end - data_start));

		#[cfg(not(feature = "timings"))]
		Ok(())
	}

	async fn serial_write(&mut self, buffer: &[u8]) -> Result<usize> {
		match self.port.write_all(buffer).await {
			Ok(_) => Ok(buffer.len()),
			// Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
			// 	println!("WARNING: serial timeout");
			// 	Ok(0)
			// }
			// Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
			// 	println!("WARNING: serial interrupted");
			// 	Ok(0)
			// }
			Err(e) => Err(e.into()),
		}
	}
}
