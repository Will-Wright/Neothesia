//! lightguide-test — manual hardware harness for the lightguide crate.
//!
//! Drives a single driver through init + a one-shot color/off command, then
//! exits. Validation is by eyeball: cargo run, look at the keyboard.
//!
//! Usage:
//!   cargo run -p lightguide --bin lightguide-test -- --driver kk-mk2 --color 0x19
//!   cargo run -p lightguide --bin lightguide-test -- --driver kk-mk2 --off

use anyhow::{Result, bail};
use clap::Parser;

use lightguide::Driver;
use lightguide::drivers::kk_mk2::KkMk2;

#[derive(Parser, Debug)]
#[command(about = "lightguide manual hardware test harness")]
struct Args {
    /// Which driver to use. Currently supported: kk-mk2.
    #[arg(long, default_value = "kk-mk2")]
    driver: String,

    /// Palette index to set (e.g. 0x19 = light green per the SynthesiaKontrol
    /// palette). Hex or decimal. Conflicts with --off.
    #[arg(long, value_parser = parse_color_byte)]
    color: Option<u8>,

    /// If set, light only the key at this buffer position. Without --key,
    /// every key on the keyboard is lit with --color.
    /// Buffer positions on S61 MK2: 0 = leftmost (C2), 60 = rightmost (C7),
    /// middle C ≈ position 24. Out-of-range values are silently ignored.
    #[arg(long, value_parser = parse_color_byte)]
    key: Option<u8>,

    /// Turn all keys off (equivalent to --color 0 without --key).
    /// Conflicts with --color.
    #[arg(long, conflicts_with = "color")]
    off: bool,
}

fn parse_color_byte(s: &str) -> Result<u8, std::num::ParseIntError> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16)
    } else {
        s.parse::<u8>()
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut driver: Box<dyn Driver> = match args.driver.as_str() {
        "kk-mk2" => Box::new(KkMk2::open()?),
        other => bail!("unknown driver: {other:?} (supported: kk-mk2)"),
    };

    driver.init()?;

    if args.off {
        driver.set_off()?;
        println!("all LEDs: off");
    } else if let Some(key) = args.key {
        // Start with a clean buffer so other keys are dark.
        driver.set_off()?;
        let color = args.color.unwrap_or(0x19);
        driver.set_one(key, color)?;
        println!("key {key}: palette 0x{color:02X} (other keys: off)");
    } else {
        let color = args.color.unwrap_or(0x19);
        driver.set_all(color)?;
        println!("all LEDs: palette 0x{color:02X}");
    }

    Ok(())
}
