use bytemuck::{cast, cast_mut, cast_ref};
use defmt::*;
use embassy_rp::{
	peripherals::{PIN_0, PIN_1, PIN_2, PIN_3, PIN_4, PIN_5, PIN_6, PIN_7, PIO0},
	pio::{Config, Direction, FifoJoin, Instance, Pio, ShiftConfig, ShiftDirection, StateMachine},
};
use embassy_time::{Duration, Instant, Timer};
use fixed_macro::fixed;
use pio_proc::pio_asm;
use serial_ws2812_shared::{BYTES_PER_LED, MAX_BUFFER_SIZE, MAX_STRIPS};

use crate::{
	globals::{LEDs, DISPLAY_CHANNEL, RETURN_CHANNEL},
	Irqs,
};

type OutputPins = (PIN_0, PIN_1, PIN_2, PIN_3, PIN_4, PIN_5, PIN_6, PIN_7);

const RESET_DURATION: Duration = Duration::from_micros(280);

#[embassy_executor::task]
pub async fn parallel_led_task(pio: PIO0, outputs: OutputPins) {
	info!("Hello from LED task on core 1");

	let mut sm = setup_ws2812_pio(pio, outputs);

	// allocate as u32 for correct byte alignment
	let mut out_buf: [u8; MAX_BUFFER_SIZE] = cast([0u32; MAX_BUFFER_SIZE / 4]);

	let mut last_write = Instant::now();
	loop {
		info!("ws2812: waiting for data pointer");
		let (num_leds, leds) = DISPLAY_CHANNEL.receive().await;

		// make sure we wait long enough for the ws2812 chips to reset
		let diff = Instant::now() - last_write;
		if diff < RESET_DURATION {
			Timer::after(RESET_DURATION - diff).await;
		}

		info!("ws2812: got data pointer, writing to GPIO");
		write_data_direct(&mut sm, leds, num_leds, &mut out_buf).await;

		info!("ws2812: done writing to GPIO, returning data pointer");
		RETURN_CHANNEL.send(leds).await;

		while !sm.tx().empty() {
			Timer::after(Duration::from_micros(5)).await;
		}
		last_write = Instant::now();
	}
}

async fn write_data_direct<PIO: Instance>(
	sm: &mut StateMachine<'_, PIO, 0>,
	leds: &LEDs,
	to_write: usize,
	out: &mut [u8; MAX_BUFFER_SIZE],
) {
	let mut current;
	let mut written_bytes = 0;

	let leds_to_write = to_write.min(leds[0].len());
	let tx = sm.tx();

	for i in 0..leds_to_write {
		let byte_idx = BYTES_PER_LED * MAX_STRIPS * i;

		// G R B, not R G B
		for (j, color) in [1, 0, 2].into_iter().enumerate() {
			current = [
				leds[0][i][color],
				leds[1][i][color],
				leds[2][i][color],
				leds[3][i][color],
				leds[4][i][color],
				leds[5][i][color],
				leds[6][i][color],
				leds[7][i][color],
			];
			let start_index = byte_idx + j * 8;

			compress_byte(&mut current, &mut out[start_index..start_index + 8]);
		}

		while byte_idx - written_bytes >= 4 && !tx.full() {
			tx.push(u32::from_be_bytes([
				out[written_bytes],
				out[written_bytes + 1],
				out[written_bytes + 2],
				out[written_bytes + 3],
			]));
			written_bytes += 4;
		}
	}

	let mut total_to_write = BYTES_PER_LED * MAX_STRIPS * leds_to_write;
	// make sure alignment is correct
	if total_to_write % 4 != 0 {
		total_to_write += 4 - total_to_write % 4;
	}

	while total_to_write - written_bytes >= 4 {
		if tx.try_push(u32::from_be_bytes([
			out[written_bytes],
			out[written_bytes + 1],
			out[written_bytes + 2],
			out[written_bytes + 3],
		])) {
			written_bytes += 4;
		}
	}
}

fn setup_ws2812_pio<'a>(pio: PIO0, outputs: OutputPins) -> StateMachine<'a, PIO0, 0> {
	let Pio {
		mut common,
		sm0: mut sm,
		..
	} = Pio::new(pio, Irqs);

	let pins = [
		&common.make_pio_pin(outputs.0),
		&common.make_pio_pin(outputs.1),
		&common.make_pio_pin(outputs.2),
		&common.make_pio_pin(outputs.3),
		&common.make_pio_pin(outputs.4),
		&common.make_pio_pin(outputs.5),
		&common.make_pio_pin(outputs.6),
		&common.make_pio_pin(outputs.7),
	];

	sm.set_pin_dirs(Direction::Out, &pins);

	// adapted from https://mcuoneclipse.com/2023/04/02/rp2040-with-pio-and-dma-to-address-ws2812b-leds/
	let prg = pio_asm!(
		"
			.wrap_target
				mov x, null           ; [1] clear X scratch register
				out x, 8              ; [1] copy 8bits from OSR to X
				mov pins, !null [2]   ; [3] T1: set all pins HIGH (!NULL)
				mov pins, x     [3]   ; [4] T2: pulse width: keep pins high (for 1 bits) or pull low (for 0 bits)
				mov pins, null        ; [1] T3: pull all pins low
			.wrap
		"
	);

	const CYCLES_PER_BIT: u32 = 1 + 1 + 3 + 4 + 1;

	let mut cfg = Config::default();
	cfg.use_program(&common.load_program(&prg.program), &[]);

	// sys clk freq: overclocked in main.rs
	let clock_freq = fixed!(266_000: U24F8);
	let ws2812_freq = fixed!(800: U24F8);
	let bit_freq = ws2812_freq * CYCLES_PER_BIT;

	cfg.clock_divider = clock_freq / bit_freq;

	cfg.shift_out = ShiftConfig {
		auto_fill: true,
		threshold: 32,
		direction: ShiftDirection::Left,
	};

	cfg.fifo_join = FifoJoin::TxOnly;

	cfg.set_out_pins(&pins);
	cfg.set_set_pins(&pins[0..4]);

	sm.set_config(&cfg);
	sm.set_enable(true);

	sm
}

/// splits bytes by bits
/// nth bit of each byte is combined into the nth byte
#[inline]
pub fn compress_byte(i: &mut [u8; 8], out: &mut [u8]) {
	for bit in out.iter_mut() {
		*bit = compress_bit(i);

		shift(i)
	}
}

#[inline]
pub fn compress_bit(i: &[u8; 8]) -> u8 {
	let [lower, upper] = cast_ref::<[u8; 8], [u32; 2]>(i);
	let lower = lower & 0x80_80_80_80_u32;
	let upper = upper & 0x80_80_80_80_u32;

	let merge = upper | (lower >> 4);
	let merge = merge | ((merge >> 2) << 16);
	let merge = merge | ((merge >> 1) << 8);

	u32::to_be_bytes(merge)[0]
}

#[inline]
fn shift(i: &mut [u8; 8]) {
	let [lower, upper] = cast_mut::<[u8; 8], [u32; 2]>(i);
	// let [lower, upper] = unsafe { transmute::<&mut [u8; 8], &mut [u32; 2]>(i) };
	*lower <<= 1;
	*upper <<= 1;
}
