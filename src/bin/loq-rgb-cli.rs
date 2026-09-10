//! `loq-rgb-cli` — command-line control for the LOQ 4-zone keyboard RGB.
//!
//! Design: the CLI shares the exact same library code as the GUI
//! (controller, packet builder, config store) so scripted behaviour can
//! never diverge from what the GUI does.

use std::io::Read;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use loq_rgb::backend::mock::MockBackend;
use loq_rgb::config::ConfigStore;
use loq_rgb::controller::Controller;
use loq_rgb::detect;
use loq_rgb::effects;
use loq_rgb::error::Error;
use loq_rgb::model::{Effect, LightingConfig, Rgb, ZONE_COUNT};

const UDEV_RULE: &str = r#"# loq-rgb: rootless access to the Lenovo 4-zone ITE RGB keyboard
# controller (LOQ / Legion / IdeaPad Gaming, 2020-2024).
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c993", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c983", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c995", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c994", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c985", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c984", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c975", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c973", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c965", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c963", MODE="0666", TAG+="uaccess"
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="048d", ATTRS{idProduct}=="c955", MODE="0666", TAG+="uaccess"
"#;

const UDEV_TARGET: &str = "/etc/udev/rules.d/60-loq-rgb.rules";

/// Autostart entry body. `Exec` is filled in with the absolute path of the
/// running binary so it works regardless of the login `PATH` (desktop
/// sessions do not necessarily include `~/.local/bin`).
fn autostart_desktop_file(exe: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=LOQ RGB\n\
         Comment=Applies your keyboard RGB profile at login, keeps the colour wave \
         running and handles Fn+Space profile cycling\n\
         Exec={} listen-hotkeys\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         X-KDE-autostart-after=panel\n",
        exe.display()
    )
}

/// `~/.config/autostart` (respecting `XDG_CONFIG_HOME`).
fn autostart_dir() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return std::path::PathBuf::from(xdg).join("autostart");
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return std::path::PathBuf::from(home)
            .join(".config")
            .join("autostart");
    }
    std::path::PathBuf::from("autostart")
}

fn autostart_entry_path() -> std::path::PathBuf {
    autostart_dir().join("loq-rgb.desktop")
}

#[derive(Parser)]
#[command(
    name = "loq-rgb-cli",
    version,
    about = "Control the Lenovo LOQ/Legion 4-zone keyboard RGB lighting.",
    long_about = "Talks to the ITE 048d:c993 (LOQ 2024) family of 4-zone RGB keyboard \
                  controllers over HID feature reports. Honest by design: it only ever \
                  sends configurations the hardware can actually represent."
)]
struct Cli {
    /// Use an in-memory fake instead of the real controller (testing).
    #[arg(long, global = true, hide = true)]
    mock: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Detect and describe the keyboard controller.
    Detect,
    /// Print the lighting state the controller currently reports.
    Status,
    /// Build a configuration and apply it to the keyboard.
    Apply(ConfigArgs),
    /// Manage named profiles.
    Profiles,
    /// Permanently delete a saved profile (never changes the hardware).
    DeleteProfile {
        /// Name of the profile to delete.
        name: String,
        /// Confirm the deletion (required for non-interactive use).
        #[arg(long)]
        yes: bool,
    },
    /// Rename a saved profile (never changes the hardware).
    RenameProfile {
        /// Current profile name.
        old: String,
        /// New profile name.
        new: String,
        /// Confirm the rename (required for non-interactive use).
        #[arg(long)]
        yes: bool,
    },
    /// Print full diagnostic information for support tickets.
    Diagnose,
    /// Print the packet bytes a configuration produces (debug/verification).
    DumpPacket(ConfigArgs),
    /// Listen for Fn+Space and cycle through the saved profiles.
    ListenHotkeys,
    /// Install (or remove) the "run at login" autostart entry.
    Autostart {
        /// Remove the autostart entry instead of installing it.
        #[arg(long)]
        remove: bool,
        /// Show the entry that would be written, without writing it.
        #[arg(long)]
        print: bool,
    },
    /// Install (or print) the udev rule granting user access to the device.
    Udev {
        /// Print the rule to stdout instead of writing it.
        #[arg(long)]
        print: bool,
    },
}

#[derive(Args, Clone)]
struct ConfigArgs {
    /// Effect: off | static | breath | wave-left | wave-right | rainbow-left |
    /// rainbow-right | smooth. (default: static)
    #[arg(long, value_parser = parse_effect)]
    effect: Option<Effect>,

    /// Animation speed 1 (slowest) .. 4 (fastest). (default: 2)
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=4))]
    speed: Option<u8>,

    /// Brightness: 1 (low) or 2 (high). (default: 2)
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
    brightness: Option<u8>,

    /// Zone colours as hex (#rrggbb or rrggbb), comma separated. 1..4
    /// values; fewer than 4 repeats the last colour across remaining zones.
    #[arg(long, value_delimiter = ',')]
    colors: Vec<String>,

    /// Apply the named profile instead of the flag-built configuration.
    #[arg(long, conflicts_with_all = ["colors", "effect", "speed", "brightness"])]
    profile: Option<String>,

    /// Save the applied configuration as a profile with this name.
    #[arg(long)]
    save: Option<String>,
}

fn parse_effect(s: &str) -> Result<Effect, String> {
    let norm: String = s.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    match norm.as_str() {
        "off" => Ok(Effect::Off),
        "static" => Ok(Effect::Static),
        "breath" | "breathing" => Ok(Effect::Breath),
        "flow-left" => Ok(Effect::FlowLeft),
        "flow-right" => Ok(Effect::FlowRight),
        // Legacy names (the firmware wave/rainbow were consolidated into the
        // continuous colour wave) kept so older scripts keep working.
        "wave-left" | "rainbow-left" => Ok(Effect::FlowLeft),
        "wave-right" | "rainbow-right" => Ok(Effect::FlowRight),
        "smooth" | "smooth-flow" => Ok(Effect::Smooth),
        other => Err(format!(
            "unknown effect {other:?}; choose from: {}",
            effects::EFFECTS
                .iter()
                .map(|e| format!("\"{}\"", e.effect.to_string().replace('_', "-")))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("hint: {}", e.hint());
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Error> {
    match &cli.command {
        Command::Detect => cmd_detect(),
        Command::Status => cmd_status(cli.mock),
        Command::Apply(args) => cmd_apply(cli.mock, args),
        Command::Profiles => cmd_profiles(),
        Command::DeleteProfile { name, yes } => cmd_delete_profile(name, *yes),
        Command::RenameProfile { old, new, yes } => cmd_rename_profile(old, new, *yes),
        Command::Diagnose => cmd_diagnose(),
        Command::DumpPacket(args) => cmd_dump_packet(args),
        Command::ListenHotkeys => cmd_listen_hotkeys(),
        Command::Autostart { remove, print } => cmd_autostart(*remove, *print),
        Command::Udev { print } => cmd_udev(*print),
    }
}

fn open_controller(mock: bool) -> Result<Controller, Error> {
    if mock {
        let backend = MockBackend::new();
        Ok(Controller::new(Box::new(backend)))
    } else {
        Controller::open_hardware()
    }
}

fn cmd_detect() -> Result<(), Error> {
    match detect::detect() {
        Some(d) => {
            println!("{}", d.describe());
            println!("zones: {ZONE_COUNT} (static + breathing per-zone colours)");
            println!(
                "effects: {}",
                effects::EFFECTS
                    .iter()
                    .map(|e| e.label)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "note: the firmware applies ONE effect to the whole keyboard — per-zone effect mixing is not supported."
            );
            Ok(())
        }
        None => Err(Error::DeviceNotFound(
            "no supported controller found on the USB bus".into(),
        )),
    }
}

fn cmd_status(mock: bool) -> Result<(), Error> {
    let mut controller = open_controller(mock)?;
    println!("device: {}", controller.device_desc());
    match controller.read_state() {
        Ok(state) => {
            let eff = match state.effect {
                loq_rgb::model::LiveEffect::Known(e) => effects::info(e).label.to_string(),
                loq_rgb::model::LiveEffect::Unknown(code) => format!("unknown(0x{code:02x})"),
            };
            println!("effect:     {eff}");
            println!("speed:      {}", state.speed);
            println!("brightness: {}", state.brightness);
            for (i, z) in state.zones.iter().enumerate() {
                println!("zone {}:     {}", i + 1, z.to_hex());
            }
            println!(
                "wave flags: right={} left={}",
                state.flag_right, state.flag_left
            );
            Ok(())
        }
        Err(e) => {
            eprintln!(
                "note: no state read back from this controller ({e}); showing last applied config if any."
            );
            if let Some(cfg) = controller.last_config() {
                print_config(cfg)
            }
            Err(e)
        }
    }
}

fn build_config(args: &ConfigArgs) -> Result<LightingConfig, Error> {
    if let Some(name) = &args.profile {
        let store = ConfigStore::load()?;
        let cfg = store
            .cfg
            .profiles
            .get(name)
            .copied()
            .ok_or_else(|| Error::Config(format!("no profile named {name:?}")))?;
        return Ok(cfg.normalized());
    }

    // Bare `apply` (no config flags at all) re-applies the active profile —
    // the autostart behaviour.
    let nothing_explicit = args.effect.is_none()
        && args.speed.is_none()
        && args.brightness.is_none()
        && args.colors.is_empty();
    if nothing_explicit && args.save.is_none() {
        let store = ConfigStore::load()?;
        return Ok(store.cfg.active_config());
    }

    let mut zones = [Rgb::white(); ZONE_COUNT];
    let mut parsed: Vec<Rgb> = Vec::new();
    for c in &args.colors {
        let rgb = Rgb::from_hex(c)
            .ok_or_else(|| Error::Config(format!("invalid colour {c:?}; expected #rrggbb")))?;
        parsed.push(rgb);
    }
    if !parsed.is_empty() {
        let last = *parsed.last().expect("non-empty");
        for (i, zone) in zones.iter_mut().enumerate() {
            *zone = parsed.get(i).copied().unwrap_or(last);
        }
    }

    Ok(LightingConfig {
        effect: args.effect.unwrap_or(Effect::Static),
        speed: args.speed.unwrap_or(2),
        brightness: args.brightness.unwrap_or(2),
        zones,
    }
    .normalized())
}

fn cmd_apply(mock: bool, args: &ConfigArgs) -> Result<(), Error> {
    let cfg = build_config(args)?;

    if let Some(name) = &args.profile {
        // Switching to a stored profile: persist the selection, then apply.
        let mut store = ConfigStore::load()?;
        store.set_active_profile(name)?;
        println!("active profile: {name}");
    }
    if let Some(save) = &args.save {
        let mut store = ConfigStore::load()?;
        store.upsert_profile(save, cfg)?;
        store.set_active_profile(save)?;
        println!("saved profile: {save}");
    }

    let mut controller = open_controller(mock)?;
    controller.apply(cfg)?;
    print_config(&cfg);
    println!("applied to {}", controller.device_desc());
    Ok(())
}

fn cmd_delete_profile(name: &str, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Config(format!(
            "refusing to delete profile {name:?} without confirmation — re-run with `--yes`"
        )));
    }
    let mut store = ConfigStore::load()?;
    match loq_rgb::profiles::delete_profile(&mut store, name)? {
        Some(new_active) => {
            println!("deleted profile {name:?}");
            println!(
                "note: it was the active profile — active selection switched to {new_active:?} \
                 (the keyboard lighting was not changed)"
            );
        }
        None => println!("deleted profile {name:?}"),
    }
    Ok(())
}

fn cmd_rename_profile(old: &str, new: &str, yes: bool) -> Result<(), Error> {
    if !yes {
        return Err(Error::Config(format!(
            "refusing to rename profile {old:?} without confirmation — re-run with `--yes`"
        )));
    }
    let mut store = ConfigStore::load()?;
    loq_rgb::profiles::rename_profile(&mut store, old, new)?;
    println!("renamed profile {old:?} to {new:?} (keyboard lighting was not changed)");
    Ok(())
}

/// Full diagnostic dump for support tickets. Exits successfully even when no
/// controller is present — the information is exactly what an issue report
/// needs.
fn cmd_diagnose() -> Result<(), Error> {
    println!("== loq-rgb diagnose ==");

    println!("[system]");
    for field in [
        "sys_vendor",
        "product_name",
        "product_family",
        "board_name",
        "bios_version",
    ] {
        let value = loq_rgb::devices::dmi_field(field).unwrap_or_else(|| "?".into());
        println!("  {field}: {value}");
    }
    println!(
        "  kernel: {}",
        std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_else(|_| "?".into())
    );
    println!("  version: {}", env!("CARGO_PKG_VERSION"));

    println!("[detection]");
    match detect::detect() {
        Some(d) => println!("  found: {}", d.describe()),
        None => println!("  found: none (no supported 048d:c9xx controller on the USB bus)"),
    }

    println!("[usb bus]");
    let devices = detect::scan_usb_devices(std::path::Path::new("/sys/bus/usb/devices"));
    for (vid, pid) in devices {
        println!("  {vid:04x}:{pid:04x}");
    }

    println!("[controller interfaces & readback]");
    match hidapi::HidApi::new() {
        Err(e) => println!("  hidapi init failed: {e}"),
        Ok(api) => {
            let found_any = api
                .device_list()
                .filter(|d| {
                    loq_rgb::devices::controller_label(d.vendor_id(), d.product_id()).is_some()
                })
                .map(|d| {
                    println!(
                        "  {:04x}:{:04x} path={} usage_page=0x{:04x} usage=0x{:04x}",
                        d.vendor_id(),
                        d.product_id(),
                        d.path().to_string_lossy(),
                        d.usage_page(),
                        d.usage()
                    );
                    match d.open_device(&api) {
                        Err(e) => println!("    open failed: {e}"),
                        Ok(dev) => {
                            let mut buf = [0u8; 33];
                            buf[0] = 0xCC;
                            match dev.get_feature_report(&mut buf) {
                                Err(e) => println!("    readback failed: {e}"),
                                Ok(n) if n < 33 => println!(
                                    "    readback: {n}-byte report (state readback NOT available)"
                                ),
                                Ok(_) => println!("    readback: full 33-byte state available"),
                            }
                        }
                    }
                })
                .count();
            if found_any == 0 {
                println!("  no supported controller interfaces found");
            }
        }
    }

    println!("[udev rules]");
    for path in [
        "/usr/lib/udev/rules.d/60-loq-rgb.rules",
        "/etc/udev/rules.d/60-loq-rgb.rules",
    ] {
        println!(
            "  {path}: {}",
            if std::path::Path::new(path).exists() {
                "installed"
            } else {
                "missing"
            }
        );
    }

    println!("[config]");
    match ConfigStore::load() {
        Ok(store) => {
            println!("  path: {}", store.path.display());
            println!(
                "  profiles: {} (active: {})",
                store.cfg.profiles.len(),
                store.cfg.active_profile
            );
        }
        Err(e) => println!("  error: {e}"),
    }

    println!("[writer lock]");
    let lock_dir = loq_rgb::instance::default_lock_dir();
    match loq_rgb::instance::probe(&lock_dir) {
        Some(owner) => println!("  held by pid {}", owner.pid),
        None => println!("  not held (no background daemon running)"),
    }

    println!("== end diagnose — include this output in your issue report ==");
    Ok(())
}

fn cmd_profiles() -> Result<(), Error> {
    let store = ConfigStore::load()?;
    if store.cfg.profiles.is_empty() {
        println!("no profiles");
        return Ok(());
    }
    for (name, cfg) in &store.cfg.profiles {
        let active = if name == &store.cfg.active_profile {
            "* "
        } else {
            "  "
        };
        let zones: Vec<String> = if cfg.effect.uses_zone_colors() {
            cfg.zones.iter().map(|z| z.to_hex()).collect()
        } else {
            vec!["-".to_string(); ZONE_COUNT]
        };
        println!(
            "{active}{name}: {} (speed {}, brightness {}, zones {})",
            effects::info(cfg.effect).label,
            cfg.speed,
            cfg.brightness,
            zones.join(" ")
        );
    }
    Ok(())
}

fn cmd_dump_packet(args: &ConfigArgs) -> Result<(), Error> {
    let cfg = build_config(args)?;
    let p = loq_rgb::packet::build_packet(&cfg);
    let hex: Vec<String> = p.iter().map(|b| format!("{b:02x}")).collect();
    println!("{}", hex.join(" "));
    Ok(())
}

/// Watch the "Ideapad extra buttons" input node for Fn+Space (key code 240)
/// and cycle to the next saved profile on every press.
///
/// Verified on the LOQ 15IRX9: the key reaches the OS as EV_KEY code 240 and
/// the firmware does not cycle natively while software control is active, so
/// each press cleanly advances through the user's saved profiles.
///
/// While the active profile uses a host-rendered effect (colour flow) this
/// loop also writes animation frames at ~25 fps so flow profiles animate
/// without the GUI. The active config is re-read from disk once per second
/// so edits made in the GUI are picked up.
fn cmd_listen_hotkeys() -> Result<(), Error> {
    use std::io::Write;
    use std::time::{Duration, Instant};
    let target_name = loq_rgb::hotkey::IDEAPAD_BUTTONS_NAME;

    // Single-writer lock: the daemon is the background animator. A second
    // instance would fight the first and cause visible flicker. The guard is
    // bound for the whole function, so the lock is held while we loop and
    // released when this daemon exits.
    let lock_dir = loq_rgb::instance::default_lock_dir();
    let Some(_writer_lock) = loq_rgb::instance::WriterLock::acquire(&lock_dir)? else {
        let owner = loq_rgb::instance::probe(&lock_dir)
            .map(|o| o.pid.to_string())
            .unwrap_or_else(|| "unknown".into());
        return Err(Error::AlreadyRunning(format!(
            "pid {owner} already holds the lighting writer lock"
        )));
    };

    let mut controller: Option<Controller> = None;
    let mut active_cfg: Option<LightingConfig> = None;
    let mut last_store_check = Instant::now() - Duration::from_secs(2);
    // Host-flow animation state.
    let mut flow_phase: f32 = 0.0;
    let mut last_flow_frame: Option<Instant> = None;

    fn apply_with(controller: &mut Option<Controller>, cfg: LightingConfig) -> Result<(), Error> {
        if controller.is_none() {
            *controller = Some(open_controller(false)?);
        }
        controller.as_mut().expect("just opened").apply(cfg)
    }

    fn refresh_active(active_cfg: &mut Option<LightingConfig>) {
        match ConfigStore::load() {
            Ok(store) => {
                *active_cfg = Some(store.cfg.active_config());
            }
            Err(e) => eprintln!("note: cannot read config: {e}"),
        }
    }

    // Startup: apply the active profile (also the apply-at-login behaviour).
    refresh_active(&mut active_cfg);
    if let Some(cfg) = active_cfg {
        match apply_with(&mut controller, cfg) {
            Ok(()) => {
                println!("applied active profile");
                let _ = std::io::stdout().flush();
            }
            Err(e) => eprintln!("note: could not apply the active profile yet ({e}); will retry"),
        }
    }
    println!("listening for Fn+Space on {target_name:?} … press Ctrl+C to stop");

    let mut node = find_input_node(target_name);
    let mut file = node.as_ref().and_then(|p| open_nonblocking(p));
    let mut warned_perms = false;
    let mut buf = [0u8; 24];
    loop {
        match &mut file {
            Some(f) => match f.read(&mut buf) {
                Ok(24) => {
                    let typ = u16::from_le_bytes([buf[16], buf[17]]);
                    let code = u16::from_le_bytes([buf[18], buf[19]]);
                    let value = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
                    if typ == 0x01 /* EV_KEY */
                        && code == loq_rgb::hotkey::FN_SPACE_CODE
                        && value == 1
                    {
                        let result = (|| -> Result<(), Error> {
                            let mut store = ConfigStore::load()?;
                            let next = loq_rgb::hotkey::advance_profile(&mut store)?;
                            let cfg = store.cfg.active_config();
                            apply_with(&mut controller, cfg)?;
                            refresh_active(&mut active_cfg);
                            flow_phase = 0.0;
                            last_flow_frame = None;
                            println!("Fn+Space → profile {next:?} applied");
                            let _ = std::io::stdout().flush();
                            Ok(())
                        })();
                        if let Err(e) = result {
                            eprintln!("Fn+Space: could not advance profile: {e}");
                            eprintln!("hint: {}", e.hint());
                        }
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    // Node vanished (unlikely); re-discover and retry.
                    file = None;
                    std::thread::sleep(Duration::from_millis(500));
                }
            },
            None => {
                node = find_input_node(target_name);
                file = node.as_ref().and_then(|p| open_nonblocking(p));
                if file.is_none() {
                    if node.is_some() && !warned_perms {
                        warned_perms = true;
                        eprintln!(
                            "note: found the input node but could not open it — run \
                             `sudo loq-rgb-cli udev` then reload/trigger udev"
                        );
                    }
                    std::thread::sleep(Duration::from_secs(1));
                } else {
                    warned_perms = false;
                }
            }
        }

        // Periodic refresh picks up profiles edited in the GUI.
        if last_store_check.elapsed() >= Duration::from_secs(1) {
            last_store_check = Instant::now();
            refresh_active(&mut active_cfg);
        }

        // Host-rendered colour flow: keep the animation running headless.
        let is_flow = active_cfg
            .map(|c| c.effect.is_host_rendered())
            .unwrap_or(false);
        if is_flow {
            if let (Some(cfg), true) = (active_cfg, controller.is_some()) {
                let now = Instant::now();
                let due = match last_flow_frame {
                    Some(t) => now.duration_since(t) >= Duration::from_millis(40),
                    None => true,
                };
                if due {
                    let dt = match last_flow_frame {
                        Some(t) => now.duration_since(t).as_secs_f32().min(0.25),
                        None => 0.04,
                    };
                    last_flow_frame = Some(now);
                    flow_phase =
                        loq_rgb::flow::advance_phase(cfg.effect, cfg.speed, flow_phase, dt);
                    let frame = LightingConfig {
                        zones: loq_rgb::flow::frame_zones(&cfg, flow_phase),
                        ..cfg
                    };
                    if let Err(e) = apply_with(&mut controller, frame) {
                        // Controller missing: drop the handle so the next
                        // frame re-opens it.
                        if matches!(e, Error::DeviceNotFound(_) | Error::Io(_)) {
                            controller = None;
                        }
                        eprintln!("flow frame failed: {e}");
                    }
                }
            }
        } else {
            flow_phase = 0.0;
            last_flow_frame = None;
        }
    }
}

/// Locate the /dev/input/eventN node for an input device by its sysfs name.
fn find_input_node(target: &str) -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/input").ok()?;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("input") {
            continue;
        }
        let name_file = entry.path().join("name");
        let Ok(content) = std::fs::read_to_string(name_file) else {
            continue;
        };
        if content.trim() != target {
            continue;
        }
        let children = std::fs::read_dir(entry.path()).ok()?;
        for child in children.flatten() {
            let cn = child.file_name().to_string_lossy().to_string();
            if cn.starts_with("event") {
                return Some(format!("/dev/input/{cn}"));
            }
        }
    }
    None
}

fn open_nonblocking(path: &str) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut o = std::fs::OpenOptions::new();
    // O_NONBLOCK = 0x800 on Linux.
    o.read(true).custom_flags(0x800);
    o.open(path).ok()
}

fn cmd_autostart(remove: bool, print_only: bool) -> Result<(), Error> {
    let entry_path = autostart_entry_path();

    if print_only {
        let exe =
            std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("loq-rgb-cli"));
        println!("{}", autostart_desktop_file(&exe));
        println!("# entry path: {}", entry_path.display());
        return Ok(());
    }

    if remove {
        return match std::fs::remove_file(&entry_path) {
            Ok(()) => {
                println!("removed {}", entry_path.display());
                println!("the keyboard will no longer be configured automatically at login");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!(
                    "no autostart entry at {} (nothing to remove)",
                    entry_path.display()
                );
                Ok(())
            }
            Err(e) => Err(Error::Io(format!(
                "cannot remove {}: {e}",
                entry_path.display()
            ))),
        };
    }

    // Install: resolve this binary's absolute path so the entry does not
    // depend on the login PATH, which often differs from a terminal's.
    let exe = std::env::current_exe()
        .map_err(|e| Error::Io(format!("cannot determine the path of this binary: {e}")))?;

    std::fs::create_dir_all(autostart_dir())
        .map_err(|e| Error::Io(format!("cannot create {}: {e}", autostart_dir().display())))?;

    std::fs::write(&entry_path, autostart_desktop_file(&exe))
        .map_err(|e| Error::Io(format!("cannot write {}: {e}", entry_path.display())))?;

    println!("wrote {}", entry_path.display());
    println!("login command: {} listen-hotkeys", exe.display());
    println!();
    println!("At login this applies your active profile, keeps the colour wave");
    println!("running and enables Fn+Space profile cycling.");
    println!("It takes effect from your next login. To test it now:");
    println!("  {} listen-hotkeys", exe.display());
    Ok(())
}

fn cmd_udev(print_only: bool) -> Result<(), Error> {
    if print_only {
        print!("{UDEV_RULE}");
        return Ok(());
    }
    match std::fs::write(UDEV_TARGET, UDEV_RULE) {
        Ok(()) => {
            println!("wrote {UDEV_TARGET}");
            println!("now run: sudo udevadm control --reload-rules && sudo udevadm trigger");
            println!("then reconnect the keyboard (or reboot) so the rule applies.");
            Ok(())
        }
        Err(e) => Err(Error::PermissionDenied(format!(
            "cannot write {UDEV_TARGET}: {e} — run `sudo {} udev --print > {UDEV_TARGET}` yourself",
            std::env::args()
                .next()
                .unwrap_or_else(|| "loq-rgb-cli".into())
        ))),
    }
}

fn print_config(cfg: &LightingConfig) {
    let eff = effects::info(cfg.effect).label;
    println!(
        "effect: {eff} | speed: {} | brightness: {}",
        cfg.speed, cfg.brightness
    );
    if !cfg.effect.writes_zone_bytes() {
        println!("zone colours: not sent (this effect drives its own visuals)");
    } else if cfg.effect.is_host_rendered() {
        if loq_rgb::flow::uses_palette(cfg) {
            println!("zone colours: palette mode — these colours are the wave's blocks");
            for (i, z) in cfg.zones.iter().enumerate() {
                println!("zone {}: {}", i + 1, z.to_hex());
            }
        } else {
            println!("zone colours: identical — the colour wave shows the full colour spectrum");
        }
        println!(
            "note: colour wave is host-rendered and animates while the GUI or \
             `loq-rgb-cli listen-hotkeys` runs."
        );
    } else {
        for (i, z) in cfg.zones.iter().enumerate() {
            println!("zone {}: {}", i + 1, z.to_hex());
        }
    }
}
