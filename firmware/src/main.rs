#![no_std]
#![no_main]
// #![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]

mod globals;
mod serial;
mod ws2812;

extern crate defmt_rtt;
extern crate panic_probe;

use core::{mem::size_of, ptr::addr_of_mut};

use bytemuck::cast;
use defmt::*;
use embassy_executor::Executor;
use embassy_rp::{
	bind_interrupts,
	clocks::PllConfig,
	config::Config,
	flash::Blocking,
	multicore::{spawn_core1, Stack},
	peripherals::{PIO0, USB},
	pio::InterruptHandler as PioInterruptHandler,
	usb::{Driver, InterruptHandler as UsbInterruptHandler},
};
use serial_ws2812_shared::MAX_BUFFER_SIZE;
use static_cell::StaticCell;

use crate::{
	globals::{LEDs, RETURN_CHANNEL},
	serial::usb_serial_task,
	ws2812::parallel_led_task,
};

bind_interrupts!(struct Irqs {
	USBCTRL_IRQ => UsbInterruptHandler<USB>;
	PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
});

const FLASH_JEDEC_BYTES: usize = size_of::<u32>();
const FLASH_ID_BYTES: usize = 16;
const ID_BYTES: usize = FLASH_JEDEC_BYTES + FLASH_ID_BYTES;
const FLASH_SIZE: usize = 2 * 1024 * 1024;

static mut CORE1_STACK: Stack<4096> = Stack::new();
static EXECUTOR0: StaticCell<Executor> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

#[cortex_m_rt::entry]
fn main() -> ! {
	let mut config = Config::default();
	let xosc = config.clocks.xosc.as_mut().expect("this should have been configured");

	// overclock to 266Mhz
	xosc.sys_pll = Some(PllConfig {
		refdiv:    1,
		fbdiv:     133,
		post_div1: 6,
		post_div2: 1,
	});

	let p = embassy_rp::init(config);

	let mut flash = embassy_rp::flash::Flash::<_, Blocking, FLASH_SIZE>::new_blocking(p.FLASH);
	let jedec = flash.blocking_jedec_id().unwrap();

	let mut id = [0; ID_BYTES];

	id[0..FLASH_JEDEC_BYTES].copy_from_slice(&jedec.to_ne_bytes());
	flash.blocking_unique_id(&mut id[FLASH_JEDEC_BYTES..]).unwrap();

	let outputs = (p.PIN_0, p.PIN_1, p.PIN_2, p.PIN_3, p.PIN_4, p.PIN_5, p.PIN_6, p.PIN_7);

	static DISPLAY_BUFFER: StaticCell<LEDs> = StaticCell::new();

	let leds = DISPLAY_BUFFER.init_with(|| cast([0u8; MAX_BUFFER_SIZE]));
	unwrap!(RETURN_CHANNEL.try_send(leds));

	let pio = p.PIO0;

	// FIXME: taking a mut reference of a static is UB
	spawn_core1(p.CORE1, unsafe { &mut *addr_of_mut!(CORE1_STACK) }, move || {
		let executor1 = EXECUTOR1.init(Executor::new());
		executor1.run(|spawner| unwrap!(spawner.spawn(parallel_led_task(pio, outputs))));
	});

	// Create the driver, from the HAL.
	let driver = Driver::new(p.USB, Irqs);

	let executor0 = EXECUTOR0.init(Executor::new());
	executor0.run(|spawner| {
		unwrap!(spawner.spawn(usb_serial_task(driver, id)));
	});
}
