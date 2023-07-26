use std::{f32::consts::PI, process, time::Instant};

use color_eyre::Result;
use eyre::eyre;
use serial_ws2812::{Config, SerialWs2812};

pub const BYTES_PER_LED: usize = 3;
pub const LEDS_PER_STRIP: usize = 512;
pub const STRIPS: usize = 8;

pub const TRANSFER_BUFFER_SIZE: usize = BYTES_PER_LED * LEDS_PER_STRIP * STRIPS;

fn main() -> Result<()> {
	color_eyre::install()?;

	let mut buffer = [0u8; TRANSFER_BUFFER_SIZE];

	let mut controller = SerialWs2812::find(Config {
		strips: STRIPS,
		leds:   LEDS_PER_STRIP,
	})?
	.ok_or(eyre!("no device found"))?;
	controller.configure()?;

	let mut frame_counter = 0;
	let mut timer = Timer::new();

	let wave_speed = 0.66;
	let wave_frequency = 64.0;
	let wave_influence = 1.0;

	let hue_speed = 0.05;

	let mut wave_offset = 0.0;
	let mut hue_offset = 0.0;

	loop {
		wave_offset += wave_speed;
		hue_offset += hue_speed;

		for led in 0..LEDS_PER_STRIP {
			let progress: f32 = ((wave_offset + LEDS_PER_STRIP as f32 - led as f32 - 1.0)
				% wave_frequency)
				/ wave_frequency * 2.0
				* PI;

			let val_top = 1.0 - (wave_influence * ((progress.sin() + 1.0) * 0.5));

			let value: [u8; 3] = HSV::new(
				(hue_offset % 255.0) as u8,
				255,
				((1.0 - val_top) * 100.0) as u8,
			)
			.into();

			let led_byte_idx = led * BYTES_PER_LED;
			for strip in 0..STRIPS {
				let strip_byte_idx = strip * LEDS_PER_STRIP * BYTES_PER_LED;
				let start_index = strip_byte_idx + led_byte_idx;
				buffer[start_index..start_index + 3].copy_from_slice(&value);
			}
		}

		let (waiting_duration, duration) = controller.send_leds(&buffer)?;

		let secs = duration.as_secs_f32();

		let bps = (buffer.len() as f32) / secs;

		let stats = timer.tick();
		if frame_counter == 0 {
			println!(
				"avg time to update: {:>6.2}ms (now {:>6.2}ms, min {:>6.2}ms, max {:>6.2}ms) - {:>7.2}kB/s - waited {:>6.2}ms - data {:>6.2}ms",
				stats.avg,
				stats.dt,
				stats.min,
				stats.max,
				bps / 1000.0,
				waiting_duration.as_micros() as f32 / 1000.0,
				duration.as_micros() as f32 / 1000.0,
			);
		}
		frame_counter = (frame_counter + 1) % 10;
	}
}

pub struct Timer {
	last:           Instant,
	moving:         [u128; 30],
	moving_min_max: [u128; 240],
	init_len:       usize,
}

pub struct Stats {
	pub dt:  f32,
	pub avg: f32,
	pub min: f32,
	pub max: f32,
}

impl Timer {
	pub fn new() -> Self {
		Timer {
			last:           Instant::now(),
			moving:         [0; 30],
			moving_min_max: [0; 240],
			init_len:       0,
		}
	}

	pub fn tick(&mut self) -> Stats {
		let current = Instant::now();
		let diff = current - self.last;
		self.last = current;

		if self.init_len < self.moving.len().max(self.moving_min_max.len()) {
			self.init_len += 1;
		}

		let diff_micros = diff.as_micros();

		self.moving.rotate_right(1);
		self.moving[0] = diff_micros;

		self.moving_min_max.rotate_right(1);
		self.moving_min_max[0] = diff_micros;

		let mut avg = 0;
		for i in self.moving.iter() {
			avg += i;
		}
		avg /= self.moving.len().min(self.init_len) as u128;

		let moving_min_max = &self.moving_min_max[..self.moving_min_max.len().min(self.init_len)];

		Stats {
			dt:  diff_micros as f32 / 1000.0,
			avg: avg as f32 / 1000.0,
			min: moving_min_max
				.iter()
				.fold(u128::MAX, |min, cur| min.min(*cur)) as f32
				/ 1000.0,
			max: moving_min_max
				.iter()
				.fold(u128::MIN, |max, cur| max.max(*cur)) as f32
				/ 1000.0,
		}
	}
}

#[derive(Copy, Clone, Debug, Default)]
pub struct RGB {
	pub r: u8,
	pub g: u8,
	pub b: u8,
}

impl RGB {
	pub fn new(r: u8, g: u8, b: u8) -> Self {
		RGB { r, g, b }
	}
}

impl Into<[u8; 3]> for RGB {
	fn into(self) -> [u8; 3] {
		[self.r, self.g, self.b]
	}
}

impl From<(u8, u8, u8)> for RGB {
	fn from(from: (u8, u8, u8)) -> Self {
		RGB::new(from.0, from.1, from.2)
	}
}

impl From<&(u8, u8, u8)> for RGB {
	fn from(from: &(u8, u8, u8)) -> Self {
		RGB::new(from.0, from.1, from.2)
	}
}

impl From<[u8; 3]> for RGB {
	fn from(from: [u8; 3]) -> Self {
		RGB::new(from[0], from[1], from[2])
	}
}

impl From<&[u8; 3]> for RGB {
	fn from(from: &[u8; 3]) -> Self {
		RGB::new(from[0], from[1], from[2])
	}
}

#[derive(Copy, Clone, Debug, Default)]
pub struct HSV {
	pub hue:        u8,
	pub saturation: u8,
	pub value:      u8,
}

impl HSV {
	pub fn new(hue: u8, saturation: u8, value: u8) -> Self {
		HSV {
			hue,
			saturation,
			value,
		}
	}

	pub fn to_rgb(self) -> (u8, u8, u8) {
		hsv2rgb_rainbow(self)
	}
}

impl Into<[u8; 3]> for HSV {
	fn into(self) -> [u8; 3] {
		let rgb: RGB = self.into();
		rgb.into()
	}
}

impl From<HSV> for RGB {
	fn from(hsv: HSV) -> Self {
		hsv.to_rgb().into()
	}
}

// from fastled
fn scale8(i: u8, scale: u8) -> u8 {
	(((i as u16) * (1 + scale as u16)) >> 8) as u8
}

// from fastled
fn scale8_video(i: u8, scale: u8) -> u8 {
	(((i as usize * scale as usize) >> 8) + if i > 0 && scale > 0 { 1 } else { 0 }) as u8
}

// from fastled
fn hsv2rgb_rainbow(hsv: HSV) -> (u8, u8, u8) {
	const K255: u8 = 255;
	const K171: u8 = 171;
	const K170: u8 = 170;
	const K85: u8 = 85;

	// Yellow has a higher inherent brightness than
	// any other color; 'pure' yellow is perceived to
	// be 93% as bright as white.  In order to make
	// yellow appear the correct relative brightness,
	// it has to be rendered brighter than all other
	// colors.
	// Level Y1 is a moderate boost, the default.
	// Level Y2 is a strong boost.
	const Y1: bool = true;
	const Y2: bool = false;

	// G2: Whether to divide all greens by two.
	// Depends GREATLY on your particular LEDs
	const G2: bool = false;

	// GSCALE: what to scale green down by.
	// Depends GREATLY on your particular LEDs
	const GSCALE: u8 = 0;

	let hue: u8 = hsv.hue;
	let sat: u8 = hsv.saturation;
	let mut val: u8 = hsv.value;

	let offset: u8 = hue & 0x1F; // 0..31

	// offset8 = offset * 8
	let mut offset8: u8 = offset;
	{
		offset8 <<= 3;
	}

	let third: u8 = scale8(offset8, (256u16 / 3) as u8); // max = 85

	let mut r = 0;
	let mut g = 0;
	let mut b = 0;

	if hue & 0x80 == 0 {
		// 0XX
		if hue & 0x40 == 0 {
			// 00X
			//section 0-1
			if hue & 0x20 == 0 {
				// 000
				//case 0: // R -> O
				r = K255 - third;
				g = third;
				b = 0;
			} else {
				// 001
				//case 1: // O -> Y
				if Y1 {
					r = K171;
					g = K85 + third;
					b = 0;
				}
				if Y2 {
					r = K170 + third;
					//uint8_t twothirds = (third << 1);
					let twothirds = scale8(offset8, ((256 * 2) / 3) as u8); // max=170
					g = K85 + twothirds;
					b = 0;
				}
			}
		} else {
			//01X
			// section 2-3
			if hue & 0x20 == 0 {
				// 010
				//case 2: // Y -> G
				if Y1 {
					//uint8_t twothirds = (third << 1);
					let twothirds = scale8(offset8, ((256 * 2) / 3) as u8); // max=170
					r = K171 - twothirds;
					g = K170 + third;
					b = 0;
				}
				if Y2 {
					r = K255 - offset8;
					g = K255;
					b = 0;
				}
			} else {
				// 011
				// case 3: // G -> A
				r = 0;
				g = K255 - third;
				b = third;
			}
		}
	} else {
		// section 4-7
		// 1XX
		if hue & 0x40 == 0 {
			// 10X
			if hue & 0x20 == 0 {
				// 100
				//case 4: // A -> B
				r = 0;
				//uint8_t twothirds = (third << 1);
				let twothirds = scale8(offset8, ((256 * 2) / 3) as u8); // max=170
				g = K171 - twothirds; //K170?
				b = K85 + twothirds;
			} else {
				// 101
				//case 5: // B -> P
				r = third;
				g = 0;

				b = K255 - third;
			}
		} else {
			if hue & 0x20 == 0 {
				// 110
				//case 6: // P -- K
				r = K85 + third;
				g = 0;

				b = K171 - third;
			} else {
				// 111
				//case 7: // K -> R
				r = K170 + third;
				g = 0;

				b = K85 - third;
			}
		}
	}

	// This is one of the good places to scale the green down,
	// although the client can scale green down as well.
	if G2 {
		g = g >> 1;
	}
	if GSCALE > 0 {
		g = scale8_video(g, GSCALE);
	}

	// Scale down colors if we're desaturated at all
	// and add the brightness_floor to r, g, and b.
	if sat != 255 {
		if sat == 0 {
			r = 255;
			b = 255;
			g = 255;
		} else {
			//nscale8x3_video( r, g, b, sat);
			if r > 0 {
				r = scale8(r, sat)
			}
			if g > 0 {
				g = scale8(g, sat)
			}
			if b > 0 {
				b = scale8(b, sat)
			}

			let mut desat = 255 - sat;
			desat = scale8(desat, desat);

			let brightness_floor = desat;
			r += brightness_floor;
			g += brightness_floor;
			b += brightness_floor;
		}
	}

	// Now scale everything down if we're at value < 255.
	if val != 255 {
		val = scale8_video(val, val);
		if val == 0 {
			r = 0;
			g = 0;
			b = 0;
		} else {
			if r > 0 {
				r = scale8(r, val)
			}
			if g > 0 {
				g = scale8(g, val)
			}
			if b > 0 {
				b = scale8(b, val)
			}
		}
	}

	(r, g, b)
}
