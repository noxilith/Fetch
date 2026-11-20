//! fetch: a donut.c-inspired fetch tool that spins your distro logo in 3D
//! with live-updating system info.

mod config;
mod gather;
mod info;
mod logo;
mod logos;
mod minimal;
mod render;
mod shading;
mod term;
mod util;

use config::{Config, Field, Item, MAX_HEIGHT};
use info::Info;
use logo::{detect_distro, distro_colors, Logo};
use render::{Camera, Colors, Raster, K2};
use shading::Shading;
use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;
use term::{Input, Terminal};
use util::{parse_integer, parse_number, truncate_bytes};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const CODENAME: &str = "Overclocked ASCII";

const HELP: &str = "\
Usage: fetch [options]

Options:
  -l, --logo <name>         Use a logo from fastfetch by name
                            Any logo fastfetch supports works, e.g.:
                              gentoo, arch, nixos, debian, ubuntu,
                              fedora, void, alpine, opensuse, manjaro,
                              proxmox, pop, linuxmint, endeavouros...
                            Run 'fastfetch --list-logos' to see all
  --rotate-x                Lock rotation to X axis only
  --rotate-y                Lock rotation to Y axis only
  -s, --speed <float>       Speed multiplier (default 1.0)
  --size <float>            Scale the logo (e.g. 2.0 for double size)
  --depth <float>           Scale the 3D depth (default 1.0)
  --height <n>              Override render height in rows
  --no-info                 Just the logo, no system info
  --no-color                Disable logo coloring
  --frames <n>              Stop after n frames (default: run until a key is pressed)
  --shading-mode <mode>     ascii (.,-~:;=!*#$@, the default), or opt into
                            sub-cell blocks: sextants (2x3) or blocks (2x2)
  --shading-chars <str>     Custom shading ramp, supports UTF-8
  --box                     Draw a border box around the info block
  --minimal                 Print the logo and info once, statically (like
                            fastfetch), then exit: no animation, least memory
  --cursor <row>,<col>      Leave the cursor visible at this screen position
                            after every frame (for a prompt drawn under the art)
  -V, --version             Show version
  -h, --help                Show this help

Config: ~/.config/fetch/config
  List field names to show (in order), one per line.
  Comment out or remove fields to hide them.
  Available fields:
    os, host, kernel, uptime, packages, shell, display, wm,
    displaymanager, theme, icons, font, cursor, terminal, cpu,
    gpu, memory, swap, disk, ip, battery, powerprofile, locale,
    colors

  Separators and custom fields:
    ---                      Blank line separator
    custom_Label=value       Static field (e.g. custom_Pronouns=he/him)

  Extra disks:
    disk=/home               Show additional mount point
    disk=/data               (repeat for multiple mounts)

  Settings:
    label_color=<color>      Label color (red, green, yellow, blue,
                             magenta, cyan, white, or ANSI number)
    separator=<char>         Title separator character
    show_title=<0/1>         Show user@host and underline (default 1)
    shading_mode=<mode>      ascii (default), blocks or sextants
    shading=<str>            Shading ramp characters
    light=<dir>              Light direction (top-left, top-right, top,
                             left, right, front, bottom-left, bottom-right)
    spin=<axes>              Rotation axes (x, y, or xy)
    speed=<float>            Rotation speed
    size=<float>             Logo scale
    height=<n>               Render height in rows

    box=<0/1>                Draw a border box around the info block

Logo: ~/.config/fetch/logo.txt
  Custom ASCII/Unicode logo. Add '# distro: <name>' as the
  first line to set the color scheme.
";

struct Cli {
    logo_name: Option<String>,
    rotate: Option<(bool, bool)>,
    speed: Option<f32>,
    show_info: bool,
    use_color: bool,
    max_frames: i64,
    shading: Option<String>,
    shading_mode: Option<String>,
    boxed: bool,
    minimal: bool,
    height: Option<i64>,
    size: Option<f32>,
    depth: Option<f32>,
    cursor: Option<(i64, i64)>,
}

enum Parsed {
    Run(Cli),
    Exit(u8),
}

fn parse_args(args: &[String]) -> Parsed {
    let mut cli = Cli {
        logo_name: None,
        rotate: None,
        speed: None,
        show_info: true,
        use_color: true,
        max_frames: 0,
        shading: None,
        shading_mode: None,
        boxed: false,
        minimal: false,
        height: None,
        size: None,
        depth: None,
        cursor: None,
    };

    let mut i = 0;
    while i < args.len() {
        let opt = args[i].as_str();
        // The value following a flag, or an error exit.
        macro_rules! value {
            () => {{
                i += 1;
                match args.get(i) {
                    Some(v) => v.as_str(),
                    None => {
                        eprintln!("fetch: option '{opt}' requires an argument");
                        return Parsed::Exit(1);
                    }
                }
            }};
        }
        macro_rules! number {
            ($min:expr, $max:expr) => {
                match parse_number(opt, value!(), $min, $max) {
                    Some(v) => v,
                    None => return Parsed::Exit(1),
                }
            };
        }
        match opt {
            "--help" | "-h" => {
                print!("{HELP}");
                return Parsed::Exit(0);
            }
            "--version" | "-V" => {
                let u = gather::uname();
                println!("fetch {VERSION} \"{CODENAME}\" ({}, {})", u.machine, u.sysname);
                return Parsed::Exit(0);
            }
            "--logo" | "-l" => cli.logo_name = Some(value!().to_string()),
            "--rotate-x" => cli.rotate = Some((true, false)),
            "--rotate-y" => cli.rotate = Some((false, true)),
            "--speed" | "-s" => cli.speed = Some(number!(-1000.0, 1000.0)),
            "--no-info" => cli.show_info = false,
            "--no-color" => cli.use_color = false,
            "--frames" => match parse_integer(opt, value!(), 0, i64::from(i32::MAX)) {
                Some(n) => cli.max_frames = n,
                None => return Parsed::Exit(1),
            },
            "--shading-chars" => cli.shading = Some(value!().to_string()),
            "--shading-mode" => cli.shading_mode = Some(value!().to_string()),
            "--height" => match parse_integer(opt, value!(), 1, MAX_HEIGHT) {
                Some(n) => cli.height = Some(n),
                None => return Parsed::Exit(1),
            },
            "--size" => cli.size = Some(number!(0.5, 5.0)),
            "--depth" => cli.depth = Some(number!(0.1, 10.0)),
            "--box" => cli.boxed = true,
            "--minimal" => cli.minimal = true,
            "--cursor" => {
                let text = value!();
                let parsed = text.split_once(',').and_then(|(r, c)| {
                    Some((
                        parse_integer(opt, r, 1, 1000)?,
                        parse_integer(opt, c, 1, 1000)?,
                    ))
                });
                match parsed {
                    Some(p) => cli.cursor = Some(p),
                    None => {
                        eprintln!("fetch: {opt} expects <row>,<col> (got '{text}')");
                        return Parsed::Exit(1);
                    }
                }
            }
            _ => {
                eprintln!("unknown option: {opt}\nTry 'fetch --help'");
                return Parsed::Exit(1);
            }
        }
        i += 1;
    }
    Parsed::Run(cli)
}

/// Pick the logo: `--logo`, then `~/.config/fetch/logo.txt`, then fastfetch for
/// the detected distro (and its ID_LIKE parents), then the built-in Gentoo one.
/// Returns the distro name used for the color scheme.
fn load_logo(logo: &mut Logo, logo_name: Option<&str>) -> String {
    if let Some(name) = logo_name {
        if !logo.load_builtin(name) && !logo.load_fastfetch(name) {
            logo.load_default();
        }
        return truncate_bytes(name, 63).to_string();
    }

    let (has_custom, file_distro) = logo.load_file();
    let mut distro = String::new();
    let mut id_like = String::new();
    if !file_distro.is_empty() {
        distro = file_distro;
    } else {
        let d = detect_distro();
        distro = if d.id.is_empty() { distro } else { d.id };
        id_like = d.id_like;
    }

    // A custom logo.txt wins, then a logo built into fetch for this distro,
    // and fastfetch only when neither applies
    let mut got_logo = has_custom || (!distro.is_empty() && logo.load_builtin(&distro));
    if !got_logo && !distro.is_empty() {
        got_logo = logo.load_fastfetch(&distro);
        if !got_logo {
            for tok in id_like.split(' ').filter(|t| !t.is_empty()) {
                if logo.load_fastfetch(tok) {
                    distro = truncate_bytes(tok, 63).to_string();
                    got_logo = true;
                    break;
                }
            }
        }
    }
    if !got_logo && logo.rows() == 0 {
        logo.load_default();
        if !distro.is_empty() && !distro.eq_ignore_ascii_case("gentoo") {
            eprintln!(
                "fetch: couldn't load {distro} logo (is fastfetch installed?). \
                 using built-in gentoo logo."
            );
        }
    }
    distro
}

/// Logo colors (outer, inner): bold magenta + white by default, then the
/// distro's scheme, then the config's `logo_outer` / `logo_inner`.
fn logo_colors(distro: &str, cfg: &Config) -> (String, String) {
    let (mut outer, mut inner) = ("\x1b[1;35m".to_string(), "\x1b[1;37m".to_string());
    if let Some((o, i)) = distro_colors(distro) {
        outer = o.into();
        inner = i.into();
    }
    if !cfg.logo_outer.is_empty() {
        outer = cfg.logo_outer.clone();
    }
    if !cfg.logo_inner.is_empty() {
        inner = cfg.logo_inner.clone();
    }
    (outer, inner)
}

/// Labels follow the logo's outer color (so the distro's scheme) unless
/// `label_color` is set.
fn resolve_label_color(cfg: &mut Config, outer: &str) {
    if cfg.label_color.is_empty() {
        cfg.label_color = outer
            .strip_prefix("\x1b[1;")
            .and_then(|s| s.strip_suffix('m'))
            .unwrap_or("35")
            .to_string();
    }
}

/// `--minimal`: one static print of the logo beside the info. Skips everything
/// the animation needs (point cloud, render buffers, terminal modes, signals).
fn run_minimal(cli: &Cli, cfg: &mut Config, term_cols: i32) -> ExitCode {
    let mut logo = Logo::default();
    let distro = load_logo(&mut logo, cli.logo_name.as_deref());
    let (color, _) = logo_colors(&distro, cfg);
    resolve_label_color(cfg, &color);

    let has_ansi = logo.raw.iter().any(|r| r.as_bytes().contains(&util::ESC));
    let rows: Vec<String> = if logo.builtin && !has_ansi {
        logo.raw.iter().map(|r| logos::flatten(r)).collect()
    } else {
        std::mem::take(&mut logo.raw)
    };

    let mut info = if cli.show_info {
        gather_info(cfg)
    } else {
        Info::new(&cfg.label_color, cfg.show_title)
    };
    if cli.boxed || cfg.boxed {
        info.box_wrap_lines();
    }

    let opts = minimal::Opts {
        use_color: cli.use_color,
        color: &color,
        term_cols,
    };
    let out = minimal::render(&rows, &info.lines, has_ansi, &opts);
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&out);
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

/// Gather all configured info lines.
fn gather_info(cfg: &Config) -> Info {
    let mut info = Info::new(&cfg.label_color, cfg.show_title);
    if cfg.show_title {
        info.add_title(&cfg.separator);
    }
    for item in &cfg.order {
        match *item {
            Item::Separator => info.add_line(""),
            Item::Custom(i) => {
                let (label, value) = &cfg.custom[i];
                info.add_info(label, value);
            }
            Item::Field(Field::Colors) => {
                info.add_line("");
                info.add_line(
                    "\x1b[40m   \x1b[41m   \x1b[42m   \x1b[43m   \
                     \x1b[44m   \x1b[45m   \x1b[46m   \x1b[47m   \x1b[0m",
                );
                info.add_line(
                    "\x1b[100m   \x1b[101m   \x1b[102m   \x1b[103m   \
                     \x1b[104m   \x1b[105m   \x1b[106m   \x1b[107m   \x1b[0m",
                );
            }
            Item::Field(f) => {
                info.current = Some(f);
                gather::gather(f, &mut info, cfg);
                info.current = None;
            }
        }
    }
    info
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cli = match parse_args(&args) {
        Parsed::Run(c) => c,
        Parsed::Exit(code) => return ExitCode::from(code),
    };
    let show_info = cli.show_info;

    // Settings: defaults, then the config file, then CLI flags on top
    let mut cfg = Config::default();
    cfg.load();
    if let Some(h) = cli.height {
        cfg.height = h;
    }
    if let Some(s) = cli.size {
        cfg.size = s;
    }
    if let Some(d) = cli.depth {
        cfg.depth = d;
        cfg.depth_user_set = true;
    }
    let (mut term_rows, mut term_cols) = term::size();

    if cli.minimal || cfg.minimal {
        return run_minimal(&cli, &mut cfg, term_cols);
    }

    // Shading: CLI flags, then config, then the ascii default
    let shading_chars = cli
        .shading
        .as_deref()
        .or(Some(cfg.shading.as_str()).filter(|s| !s.is_empty()));
    let shading_mode = cli
        .shading_mode
        .as_deref()
        .or(Some(cfg.shading_mode.as_str()).filter(|s| !s.is_empty()));
    let Some(shading) = Shading::select(shading_mode, shading_chars) else {
        eprintln!(
            "unknown shading mode: {}\nTry 'fetch --help'",
            shading_mode.unwrap_or("")
        );
        return ExitCode::from(1);
    };

    let speed = cli.speed.or(cfg.speed).unwrap_or(1.0);
    let (rotate_x, rotate_y) = cli.rotate.or(cfg.spin).unwrap_or((true, true));
    if cli.boxed {
        cfg.boxed = true;
    }

    let mut logo = Logo::default();
    let distro = load_logo(&mut logo, cli.logo_name.as_deref());
    logo.process();

    let (color_outer, color_inner) = logo_colors(&distro, &cfg);
    resolve_label_color(&mut cfg, &color_outer);

    let mut info = if show_info {
        gather_info(&cfg)
    } else {
        Info::new(&cfg.label_color, cfg.show_title)
    };
    if cfg.boxed {
        info.box_wrap_lines();
    }
    let mut layout = render::apply_layout(
        term_rows,
        term_cols,
        cfg.size,
        cfg.height,
        show_info,
        &info.lines,
    );

    let points = render::build_points(
        &logo,
        cfg.size,
        cfg.depth,
        cfg.depth_user_set,
        shading.sub_rows,
    );

    let (mut a, mut b) = (0.0f32, 0.0f32);
    let k1_for = |logo_height: i32| 37.0f32 * logo_height as f32 / 36.0 * cfg.size;
    let mut k1 = k1_for(layout.logo_height);

    // Compute the face-on (A=0, B=0) projection extent from the point cloud.
    // This is deterministic and sets a fixed frame height.
    let mut face_up = 0.0f32;
    let mut face_dn = 0.0f32;
    for i in 0..points.len() {
        let zc = points.pz[i] + K2;
        if zc < 0.1 {
            continue;
        }
        let ys = k1 * points.py[i] / zc;
        face_up = face_up.max(ys);
        face_dn = face_dn.max(-ys);
    }
    // Place y_center so the logo top lands at row 1 (row 0 is padding)
    let fixed_y_center = face_up + 1.0;

    // Tighten render_height to fit the face-on logo + info, but not when the
    // user explicitly set a height
    if cfg.height == 0 {
        let fetch_start = i32::from(show_info);
        let logo_bottom = (fixed_y_center + face_dn) as i32 + 2;
        let info_bottom = if show_info {
            fetch_start + info.lines.len() as i32
        } else {
            0
        };
        layout.render_height = layout.render_height.min(logo_bottom.max(info_bottom));
    }

    term::install_signal_handlers();
    let _terminal = Terminal::enter(cli.cursor.is_none());

    let mut raster = Raster::new();
    let mut frame_buf: Vec<u8> = Vec::new();
    let mut input = Input::default();
    let colors = Colors {
        inner: &color_inner,
        outer: &color_outer,
        use_color: cli.use_color,
        logo_has_ansi: logo.has_ansi,
    };

    let mut frame: i64 = 0;
    // Set when the user ends the animation (key or signal) rather than --frames running out
    let mut interrupted = false;
    while cli.max_frames == 0 || frame < cli.max_frames {
        if term::stop_requested() {
            interrupted = true;
            break;
        }
        // Mouse events control rotation; any other key exits.
        if input.poll(&mut a, &mut b) {
            interrupted = true;
            break;
        }
        // Terminal resize: recompute the same layout as startup
        if term::take_resized() {
            (term_rows, term_cols) = term::size();
            let old = layout;
            layout = render::apply_layout(
                term_rows,
                term_cols,
                cfg.size,
                cfg.height,
                show_info,
                &info.lines,
            );
            if layout != old {
                k1 = k1_for(layout.logo_height);
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(b"\x1b[2J");
                let _ = out.flush();
            }
        }
        // Refresh the fast dynamic fields about once a second (every 20
        // frames): they're pure /proc and /sys reads, so they don't hitch the
        // animation. Fields that run external tools stay static.
        if show_info && frame > 0 && frame % 20 == 0 {
            info.refresh = true;
            for f in [Field::Uptime, Field::Memory, Field::Swap, Field::Battery] {
                if info.field_line[f.index()].is_some() {
                    info.current = Some(f);
                    gather::refresh(f, &mut info);
                }
            }
            info.current = None;
            info.refresh = false;
        }

        if !input.dragging {
            if input.drag_vx != 0.0 || input.drag_vy != 0.0 {
                // flung: keep spinning at the release velocity
                a += input.drag_vx;
                b += input.drag_vy;
            } else {
                a += if rotate_x { 0.04 * speed } else { 0.0 };
                b += if rotate_y { 0.06 * speed } else { 0.0 };
            }
        }

        let y_center = if !layout.stacked && show_info {
            fixed_y_center
        } else {
            layout.render_height as f32 * 0.5
        };
        let cam = Camera {
            k1,
            y_center,
            light: cfg.light,
        };
        raster.draw(&points, a, b, &cam, &layout, &shading);

        let padding = render::alignment_padding(
            cfg.v_align,
            cfg.h_align,
            term_rows,
            term_cols,
            &layout,
            &info.lines,
        );
        render::compose_frame(
            &mut frame_buf,
            &raster,
            &shading,
            &layout,
            &colors,
            &info.lines,
            show_info,
            padding,
        );
        // Park the cursor (e.g. on the prompt's input line) until the next frame
        if let Some((row, col)) = cli.cursor {
            frame_buf.extend_from_slice(format!("\x1b[{row};{col}H").as_bytes());
        }
        let mut stdout = std::io::stdout().lock();
        if stdout.write_all(&frame_buf).and_then(|()| stdout.flush()).is_err() {
            break;
        }
        drop(stdout);
        std::thread::sleep(Duration::from_millis(50));
        frame += 1;
    }

    // Leave the logo face-on instead of frozen at whatever angle it was spinning through
    if interrupted {
        let y_center = if !layout.stacked && show_info {
            fixed_y_center
        } else {
            layout.render_height as f32 * 0.5
        };
        let cam = Camera {
            k1,
            y_center,
            light: cfg.light,
        };
        raster.draw(&points, 0.0, 0.0, &cam, &layout, &shading);
        let padding = render::alignment_padding(
            cfg.v_align,
            cfg.h_align,
            term_rows,
            term_cols,
            &layout,
            &info.lines,
        );
        render::compose_frame(
            &mut frame_buf, &raster, &shading, &layout, &colors, &info.lines, show_info, padding,
        );
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(&frame_buf);
        let _ = stdout.flush();
    }

    // One blank row under the art so the shell prompt doesn't sit right against it
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(b"\n\x1b[?25h");
    let _ = out.flush();
    ExitCode::SUCCESS
}
