use std::process::ExitCode;

use pcctl::openrgb::{self, Change, Client, Color, Target, DEFAULT_ADDR};
use pcctl::preset;
use pcctl::report::{Outcome, Report};

const USAGE: &str = "\
pcctl - apply lighting presets through the OpenRGB service

USAGE:
    pcctl list                                  Show devices OpenRGB controls
    pcctl profiles                              Show profiles saved in OpenRGB
    pcctl set <ram|gpu|board> <#RRGGBB>         Solid color (Direct/Static mode)
    pcctl set <ram|gpu|board> --mode <name> [<#RRGGBB>]
                                                Switch to a named mode
    pcctl apply <preset>                        Apply %APPDATA%\\pc-control\\presets\\<preset>.toml
                                                (or a path to a .toml file)

EXIT CODES:
    0  every device applied and verified
    2  some devices applied
    1  nothing applied";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[&str]) -> Result<u8, String> {
    match args {
        [] | ["help"] | ["-h"] | ["--help"] => {
            println!("{USAGE}");
            Ok(0)
        }
        ["list"] => list(),
        ["profiles"] => profiles(),
        ["set", target, rest @ ..] => set(target, rest),
        ["apply", name] => apply(name),
        _ => Err(format!("unrecognized arguments: {}\n\n{USAGE}", args.join(" "))),
    }
}

fn connect() -> Result<Client, String> {
    Client::connect(DEFAULT_ADDR).map_err(|e| e.to_string())
}

fn parse_color(s: &str) -> Result<Color, String> {
    Color::from_hex(s).ok_or_else(|| format!("\"{s}\" isn't a color; use #RRGGBB, e.g. #0000FF"))
}

fn list() -> Result<u8, String> {
    let mut client = connect()?;
    let controllers = client.controllers().map_err(|e| e.to_string())?;
    println!("OpenRGB at {DEFAULT_ADDR} (SDK protocol {})", client.version());
    for (i, c) in controllers.iter().enumerate() {
        println!("[{i}] {:<11} {}", c.device_type.label(), c.name);
        println!("     location: {}", c.location);
        let zones: Vec<String> = c.zones.iter().map(|z| format!("{} ({} LEDs)", z.name, z.leds_count)).collect();
        println!("     zones:    {}", zones.join(", "));
        println!("     mode:     {}", c.active().map_or("?", |m| m.name.as_str()));
        let modes: Vec<&str> = c.modes.iter().map(|m| m.name.as_str()).collect();
        println!("     modes:    {}", modes.join(", "));
        let savable: Vec<&str> = c.modes.iter().filter(|m| m.can_save()).map(|m| m.name.as_str()).collect();
        println!("     can save to device: {}", if savable.is_empty() { "no".to_string() } else { savable.join(", ") });
    }
    Ok(0)
}

fn profiles() -> Result<u8, String> {
    let mut client = connect()?;
    let names = client.profiles().map_err(|e| e.to_string())?;
    if names.is_empty() {
        println!("(no profiles saved in OpenRGB)");
    }
    for name in names {
        println!("{name}");
    }
    Ok(0)
}

fn set(target: &str, rest: &[&str]) -> Result<u8, String> {
    let target = Target::parse(target).ok_or_else(|| format!("unknown device \"{target}\"; use ram, gpu or board"))?;
    let change = match rest {
        [color] => Change::Color(parse_color(color)?),
        ["--mode", name] => Change::Mode { name: name.to_string(), color: None },
        ["--mode", name, color] => Change::Mode { name: name.to_string(), color: Some(parse_color(color)?) },
        _ => return Err(format!("expected a color or --mode <name>\n\n{USAGE}")),
    };
    let mut report = Report::default();
    let outcome = match connect() {
        Ok(mut client) => openrgb::apply(&mut client, target, &change),
        Err(e) => Outcome::Failed(e),
    };
    report.push(target.label(), outcome);
    print!("{report}");
    Ok(report.exit_code())
}

fn apply(name: &str) -> Result<u8, String> {
    let path = preset::resolve_path(name).map_err(|e| e.to_string())?;
    let preset = preset::load(&path).map_err(|e| e.to_string())?;
    let mut report = Report::default();
    if let Some(profile) = &preset.openrgb_profile {
        let outcome = match connect() {
            Ok(mut client) => openrgb::apply_profile(&mut client, profile),
            Err(e) => Outcome::Failed(e),
        };
        report.push("lighting", outcome);
    }
    print!("{report}");
    Ok(report.exit_code())
}
