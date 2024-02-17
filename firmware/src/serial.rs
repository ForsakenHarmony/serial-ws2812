use core::str::from_utf8;

use bytemuck::cast_slice;
use defmt::info;
use embassy_rp::{
	peripherals::USB,
	usb::{Driver, Instance},
};
use embassy_usb::{class::cdc_acm, driver::EndpointError, Builder};
use futures::future;
use serial_ws2812_shared::{
	BYTES_PER_LED,
	DEVICE_ERROR_MESSAGE,
	DEVICE_MANUFACTURER,
	DEVICE_OK_MESSAGE,
	DEVICE_PARTIAL_MESSAGE,
	DEVICE_PRODUCT_ID,
	DEVICE_PRODUCT_NAME,
	DEVICE_VENDOR_ID,
	MAX_BUFFER_SIZE,
	MAX_LEDS_PER_STRIP,
	MAX_STRIPS,
	MESSAGE_NUM_LEN,
	MESSAGE_TYPE_LEN,
	SET_LEDS_MESSAGE,
	SET_STRIPS_MESSAGE,
	UPDATE_MESSAGE,
};

use crate::{
	globals::{DISPLAY_CHANNEL, RETURN_CHANNEL},
	ID_BYTES,
};

const PACKET_LEN: u8 = 64;

#[embassy_executor::task]
pub async fn usb_serial_task(driver: Driver<'static, USB>, id: [u8; ID_BYTES]) {
	info!("Hello from USB task on core 0");

	let mut serial = [0; ID_BYTES * 2];
	for (i, byte) in id.into_iter().enumerate() {
		for j in 0..2 {
			let nibble = (byte >> (4 - 4 * (j & 1))) & 0xf;
			serial[i * 2 + j] = if nibble < 10 { nibble + b'0' } else { nibble + b'A' - 10 };
		}
	}

	// Create embassy-usb Config
	let mut config = embassy_usb::Config::new(DEVICE_VENDOR_ID, DEVICE_PRODUCT_ID);
	config.manufacturer = Some(DEVICE_MANUFACTURER);
	config.product = Some(DEVICE_PRODUCT_NAME);
	config.serial_number = Some(from_utf8(&serial).unwrap());
	config.max_power = 100;
	config.max_packet_size_0 = PACKET_LEN;

	// Required for windows compatiblity.
	// https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
	config.device_class = 0xEF;
	config.device_sub_class = 0x02;
	config.device_protocol = 0x01;
	config.composite_with_iads = true;

	// Create embassy-usb DeviceBuilder using the driver and config.
	// It needs some buffers for building the descriptors.
	let mut device_descriptor = [0; 256];
	let mut config_descriptor = [0; 256];
	let mut bos_descriptor = [0; 256];
	let mut control_buf = [0; 128];

	let mut state = cdc_acm::State::new();

	let mut builder = Builder::new(
		driver,
		config,
		&mut device_descriptor,
		&mut config_descriptor,
		&mut bos_descriptor,
		&mut control_buf,
	);

	let mut class = cdc_acm::CdcAcmClass::new(&mut builder, &mut state, 64);

	let mut usb = builder.build();

	future::join(
		async {
			loop {
				usb.run().await;
			}
		},
		async {
			loop {
				class.wait_connection().await;
				info!("Connected");
				let _ = read_serial(&mut class).await;
				info!("Disconnected");
			}
		},
	)
	.await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
	fn from(val: EndpointError) -> Self {
		match val {
			EndpointError::BufferOverflow => panic!("Buffer overflow"),
			EndpointError::Disabled => Disconnected {},
		}
	}
}

enum Command {
	Update,
	SetStrips,
	SetLeds,
}

struct Config {
	strips: usize,
	leds:   usize,
}

async fn read_serial<'d, T: Instance + 'd>(
	class: &mut cdc_acm::CdcAcmClass<'d, Driver<'d, T>>,
) -> Result<(), Disconnected> {
	let mut buf = [0; MESSAGE_TYPE_LEN + MAX_BUFFER_SIZE + PACKET_LEN as usize];
	let mut idx = 0;
	let mut command = None;

	let mut cfg = Config { strips: 3, leds: 512 };

	loop {
		idx += class.read_packet(&mut buf[idx..]).await?;
		let buf = &buf[..idx];
		if buf.len() < 8 {
			continue;
		}

		if command.is_none() {
			let incoming = &buf[..8];
			let new_command = if incoming == UPDATE_MESSAGE {
				info!("received update command :)");

				class.write_packet(DEVICE_PARTIAL_MESSAGE).await?;
				Command::Update
			} else if incoming == SET_STRIPS_MESSAGE {
				info!("received set strips command :)");

				class.write_packet(DEVICE_PARTIAL_MESSAGE).await?;
				Command::SetStrips
			} else if incoming == SET_LEDS_MESSAGE {
				info!("received set leds command :)");

				class.write_packet(DEVICE_PARTIAL_MESSAGE).await?;
				Command::SetLeds
			} else {
				info!("received invalid command :(");

				class.write_packet(DEVICE_ERROR_MESSAGE).await?;
				idx = 0;
				continue;
			};

			command = Some(new_command);
		}

		match command {
			None => {
				unreachable!();
			}
			Some(Command::SetLeds) if buf.len() >= MESSAGE_TYPE_LEN + MESSAGE_NUM_LEN => {
				let num = usize::from_le_bytes([
					buf[MESSAGE_TYPE_LEN],
					buf[MESSAGE_TYPE_LEN + 1],
					buf[MESSAGE_TYPE_LEN + 2],
					buf[MESSAGE_TYPE_LEN + 3],
				]);

				if num > MAX_LEDS_PER_STRIP {
					class.write_packet(DEVICE_ERROR_MESSAGE).await?;
					continue;
				}

				class.write_packet(DEVICE_OK_MESSAGE).await?;

				cfg.leds = num;
			}
			Some(Command::SetStrips) if buf.len() >= MESSAGE_TYPE_LEN + MESSAGE_NUM_LEN => {
				let num = usize::from_le_bytes([
					buf[MESSAGE_TYPE_LEN],
					buf[MESSAGE_TYPE_LEN + 1],
					buf[MESSAGE_TYPE_LEN + 2],
					buf[MESSAGE_TYPE_LEN + 3],
				]);

				if num > MAX_STRIPS {
					class.write_packet(DEVICE_ERROR_MESSAGE).await?;
					continue;
				}

				class.write_packet(DEVICE_OK_MESSAGE).await?;

				cfg.strips = num;
			}
			Some(Command::Update) if buf.len() >= MESSAGE_TYPE_LEN + BYTES_PER_LED * cfg.leds * cfg.strips => {
				class.write_packet(DEVICE_OK_MESSAGE).await?;

				info!("update command data received, waiting for data pointer");
				let leds = RETURN_CHANNEL.recv().await;
				info!("data pointer received");

				let data = &buf[MESSAGE_TYPE_LEN..];
				for (i, strip) in leds.iter_mut().enumerate().take(cfg.strips) {
					let start_idx = i * cfg.leds * BYTES_PER_LED;
					strip[..cfg.leds]
						.copy_from_slice(cast_slice(&data[start_idx..start_idx + cfg.leds * BYTES_PER_LED]));
				}

				DISPLAY_CHANNEL.send((cfg.leds, leds)).await;
				info!("sent data pointer to leds");
			}
			_ => {
				continue;
			}
		}

		command = None;
		idx = 0;
	}
}
