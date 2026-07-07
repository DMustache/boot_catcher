#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::env;
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

type Bool = i32;
type Dword = u32;
type Long = i32;
type Uint = u32;
type Word = u16;
type Hwnd = *mut c_void;
type Hdc = *mut c_void;
type Hbitmap = *mut c_void;
type Hgdiobj = *mut c_void;

const SRCCOPY: Dword = 0x00CC0020;
const DIB_RGB_COLORS: Uint = 0;
const BI_RGB: Dword = 0;
const INPUT_KEYBOARD: Dword = 1;
const KEYEVENTF_KEYUP: Dword = 0x0002;
const MOUSEEVENTF_LEFTDOWN: Dword = 0x0002;
const MOUSEEVENTF_LEFTUP: Dword = 0x0004;
const VK_A: Word = 0x41;
const VK_D: Word = 0x44;
const VK_SPACE: Word = 0x20;
static STOP: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: Long,
    y: Long,
}

#[repr(C)]
struct BitmapInfoHeader {
    bi_size: Dword,
    bi_width: Long,
    bi_height: Long,
    bi_planes: Word,
    bi_bit_count: Word,
    bi_compression: Dword,
    bi_size_image: Dword,
    bi_x_pels_per_meter: Long,
    bi_y_pels_per_meter: Long,
    bi_clr_used: Dword,
    bi_clr_important: Dword,
}

#[repr(C)]
struct BitmapInfo {
    bmi_header: BitmapInfoHeader,
    bmi_colors: [Dword; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KeyBdInput {
    w_vk: Word,
    w_scan: Word,
    dw_flags: Dword,
    time: Dword,
    dw_extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Input {
    r#type: Dword,
    _pad: Dword,
    ki: KeyBdInput,
    _union_pad: u64,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetCursorPos(lp_point: *mut Point) -> Bool;
    fn SetCursorPos(x: i32, y: i32) -> Bool;
    fn mouse_event(dw_flags: Dword, dx: Dword, dy: Dword, dw_data: Dword, dw_extra_info: usize);
    fn GetDC(h_wnd: Hwnd) -> Hdc;
    fn ReleaseDC(h_wnd: Hwnd, h_dc: Hdc) -> i32;
    fn SendInput(c_inputs: Uint, p_inputs: *const Input, cb_size: i32) -> Uint;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(hdc: Hdc) -> Hdc;
    fn CreateCompatibleBitmap(hdc: Hdc, cx: i32, cy: i32) -> Hbitmap;
    fn SelectObject(hdc: Hdc, h: Hgdiobj) -> Hgdiobj;
    fn BitBlt(hdc: Hdc, x: i32, y: i32, cx: i32, cy: i32, hdc_src: Hdc, x1: i32, y1: i32, rop: Dword) -> Bool;
    fn GetDIBits(
        hdc: Hdc,
        hbm: Hbitmap,
        start: Uint,
        c_lines: Uint,
        lpv_bits: *mut c_void,
        lpbmi: *mut BitmapInfo,
        usage: Uint,
    ) -> i32;
    fn DeleteObject(ho: Hgdiobj) -> Bool;
    fn DeleteDC(hdc: Hdc) -> Bool;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(Dword) -> Bool>, add: Bool) -> Bool;
}

unsafe extern "system" fn console_ctrl_handler(_ctrl_type: Dword) -> Bool {
    STOP.store(true, Ordering::SeqCst);
    1
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[derive(Clone, Copy, Debug)]
struct Roi {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Clone, Copy, Debug)]
struct BootSize {
    w: i32,
    h: i32,
}

#[derive(Clone, Copy, Debug)]
struct Detection {
    x: i32,
    y: i32,
    score: f32,
    bbox: Roi,
    source: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct MotionState {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    last_t: f64,
    lost_frames: i32,
    seen_frames: i32,
    score: f32,
}

impl MotionState {
    fn speed(self) -> f32 {
        (self.vx * self.vx + self.vy * self.vy).sqrt()
    }
}

#[derive(Clone, Debug)]
struct Args {
    fps: f64,
    log_file: String,
    template: String,
    search_top: f32,
    search_bottom: f32,
    search_margin: f32,
    cart_y: f32,
    cart_speed: f32,
    deadzone: f32,
    min_boot_speed: f32,
    lost_keep_frames: i32,
    roi_padding: i32,
    roi_speed: f32,
    lost_roi_grow: i32,
    boot_size_max_scale: f32,
    intercept_horizon: f32,
    physics_step: f32,
    boot_radius: i32,
    rescue_space_delay: f64,
    rescue_space_taps: i32,
    rescue_space_interval: f64,
    rescue_space_cooldown: f64,
    no_rescue_space: bool,
    debug: bool,
    no_startup: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            fps: 60.0,
            log_file: "boot_catcher_debug.tsv".to_string(),
            template: "boot_flying_template.png".to_string(),
            search_top: 0.15,
            search_bottom: 0.96,
            search_margin: 0.04,
            cart_y: 0.89,
            cart_speed: 430.0,
            deadzone: 18.0,
            min_boot_speed: 180.0,
            lost_keep_frames: 20,
            roi_padding: 28,
            roi_speed: 0.045,
            lost_roi_grow: 14,
            boot_size_max_scale: 1.45,
            intercept_horizon: 2.2,
            physics_step: 0.018,
            boot_radius: 16,
            rescue_space_delay: 2.0,
            rescue_space_taps: 2,
            rescue_space_interval: 0.35,
            rescue_space_cooldown: 3.0,
            no_rescue_space: false,
            debug: false,
            no_startup: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HeldKey {
    None,
    A,
    D,
}

struct ScreenCapture {
    screen_dc: Hdc,
    mem_dc: Hdc,
    bitmap: Hbitmap,
    old_obj: Hgdiobj,
    width: i32,
    height: i32,
    buf: Vec<u8>,
}

impl ScreenCapture {
    fn new(width: i32, height: i32) -> io::Result<Self> {
        unsafe {
            let screen_dc = GetDC(std::ptr::null_mut());
            if screen_dc.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mem_dc = CreateCompatibleDC(screen_dc);
            if mem_dc.is_null() {
                ReleaseDC(std::ptr::null_mut(), screen_dc);
                return Err(io::Error::last_os_error());
            }
            let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
            if bitmap.is_null() {
                DeleteDC(mem_dc);
                ReleaseDC(std::ptr::null_mut(), screen_dc);
                return Err(io::Error::last_os_error());
            }
            let old_obj = SelectObject(mem_dc, bitmap as Hgdiobj);
            Ok(Self {
                screen_dc,
                mem_dc,
                bitmap,
                old_obj,
                width,
                height,
                buf: vec![0; (width * height * 4) as usize],
            })
        }
    }

    fn grab_bgra(&mut self, rect: Rect) -> io::Result<&[u8]> {
        unsafe {
            if BitBlt(
                self.mem_dc,
                0,
                0,
                self.width,
                self.height,
                self.screen_dc,
                rect.left,
                rect.top,
                SRCCOPY,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut info = BitmapInfo {
                bmi_header: BitmapInfoHeader {
                    bi_size: std::mem::size_of::<BitmapInfoHeader>() as Dword,
                    bi_width: self.width,
                    bi_height: -self.height,
                    bi_planes: 1,
                    bi_bit_count: 32,
                    bi_compression: BI_RGB,
                    bi_size_image: (self.width * self.height * 4) as Dword,
                    bi_x_pels_per_meter: 0,
                    bi_y_pels_per_meter: 0,
                    bi_clr_used: 0,
                    bi_clr_important: 0,
                },
                bmi_colors: [0],
            };
            let got = GetDIBits(
                self.mem_dc,
                self.bitmap,
                0,
                self.height as Uint,
                self.buf.as_mut_ptr() as *mut c_void,
                &mut info,
                DIB_RGB_COLORS,
            );
            if got == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(&self.buf)
        }
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old_obj);
            DeleteObject(self.bitmap as Hgdiobj);
            DeleteDC(self.mem_dc);
            ReleaseDC(std::ptr::null_mut(), self.screen_dc);
        }
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--fps" => args.fps = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.fps),
            "--log-file" => args.log_file = it.next().unwrap_or_default(),
            "--template" => args.template = it.next().unwrap_or(args.template),
            "--debug" => args.debug = true,
            "--no-startup" => args.no_startup = true,
            "--no-rescue-space" => args.no_rescue_space = true,
            "--rescue-space-delay" => args.rescue_space_delay = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.rescue_space_delay),
            "--rescue-space-taps" => args.rescue_space_taps = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.rescue_space_taps),
            "--rescue-space-interval" => args.rescue_space_interval = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.rescue_space_interval),
            "--rescue-space-cooldown" => args.rescue_space_cooldown = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.rescue_space_cooldown),
            "--deadzone" => args.deadzone = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.deadzone),
            "--cart-speed" => args.cart_speed = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.cart_speed),
            "--min-boot-speed" => args.min_boot_speed = it.next().and_then(|v| v.parse().ok()).unwrap_or(args.min_boot_speed),
            "--help" | "-h" => {
                println!("Usage: boot_catcher_rs [--fps N] [--debug] [--log-file PATH] [--no-startup] [--no-rescue-space]");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    args
}

fn cursor_pos() -> io::Result<Point> {
    let mut p = Point { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut p) == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(p)
        }
    }
}

fn wait_for_still_mouse(label: &str, seconds: f64, radius: f64) -> io::Result<Point> {
    println!("{label}");
    println!("Move mouse there and keep it still for {seconds:.1}s. Ctrl+C cancels.");
    let mut base = cursor_pos()?;
    let mut stable_since = Instant::now();
    loop {
        sleep(Duration::from_millis(50));
        let p = cursor_pos()?;
        let dx = (p.x - base.x) as f64;
        let dy = (p.y - base.y) as f64;
        if (dx * dx + dy * dy).sqrt() <= radius {
            if stable_since.elapsed().as_secs_f64() >= seconds {
                return Ok(p);
            }
        } else {
            base = p;
            stable_since = Instant::now();
        }
    }
}

fn calibrate_game_rect() -> io::Result<Rect> {
    let p1 = wait_for_still_mouse("Top-left corner of game field", 3.0, 4.0)?;
    let p2 = wait_for_still_mouse("Bottom-right corner of game field", 3.0, 4.0)?;
    let left = p1.x.min(p2.x);
    let top = p1.y.min(p2.y);
    let right = p1.x.max(p2.x);
    let bottom = p1.y.max(p2.y);
    let rect = Rect {
        left,
        top,
        width: right - left,
        height: bottom - top,
    };
    if rect.width < 250 || rect.height < 350 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "game rect is too small"));
    }
    println!("Game rect: left={} top={} width={} height={}", rect.left, rect.top, rect.width, rect.height);
    Ok(rect)
}

fn read_png_size(path: &str) -> io::Result<BootSize> {
    let mut f = File::open(path)?;
    let mut header = [0u8; 24];
    f.read_exact(&mut header)?;
    let png_sig = [137, 80, 78, 71, 13, 10, 26, 10];
    if header[0..8] != png_sig || &header[12..16] != b"IHDR" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "template must be PNG"));
    }
    let w = i32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let h = i32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    Ok(BootSize { w, h })
}

fn clamp_f32(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

fn base_search_roi(width: i32, height: i32, args: &Args) -> Roi {
    let x0 = (width as f32 * args.search_margin) as i32;
    let x1 = (width as f32 * (1.0 - args.search_margin)) as i32;
    let y0 = (height as f32 * args.search_top) as i32;
    let y1 = (height as f32 * args.search_bottom) as i32;
    Roi { x: x0, y: y0, w: (x1 - x0).max(1), h: (y1 - y0).max(1) }
}

fn intersect(a: Roi, b: Roi) -> Option<Roi> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        None
    } else {
        Some(Roi { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }
}

fn roi_around(x: f32, y: f32, radius: i32, width: i32, height: i32, base: Roi) -> Roi {
    let local = Roi {
        x: (x - radius as f32).round() as i32,
        y: (y - radius as f32).round() as i32,
        w: radius * 2,
        h: radius * 2,
    };
    let screen = Roi { x: 0, y: 0, w: width, h: height };
    intersect(local, screen).and_then(|r| intersect(r, base)).unwrap_or(base)
}

fn idx(width: i32, x: i32, y: i32) -> usize {
    ((y * width + x) * 4) as usize
}

fn bgra_to_gray(frame: &[u8], width: i32, height: i32, out: &mut Vec<u8>) {
    out.resize((width * height) as usize, 0);
    for y in 0..height {
        for x in 0..width {
            let i = idx(width, x, y);
            let b = frame[i] as u32;
            let g = frame[i + 1] as u32;
            let r = frame[i + 2] as u32;
            out[(y * width + x) as usize] = ((77 * r + 150 * g + 29 * b) >> 8) as u8;
        }
    }
}

fn hsv_like(r: u8, g: u8, b: u8) -> (i32, i32, i32) {
    let r = r as i32;
    let g = g as i32;
    let b = b as i32;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = if max == 0 { 0 } else { d * 255 / max };
    let mut h = if d == 0 {
        0
    } else if max == r {
        30 * (g - b) / d
    } else if max == g {
        60 + 30 * (b - r) / d
    } else {
        120 + 30 * (r - g) / d
    };
    if h < 0 {
        h += 180;
    }
    (h, s, max)
}

fn boot_color(frame: &[u8], width: i32, x: i32, y: i32) -> bool {
    let i = idx(width, x, y);
    let (h, s, v) = hsv_like(frame[i + 2], frame[i + 1], frame[i]);
    (3..=36).contains(&h) && s >= 35 && v >= 30 && v <= 245
}

fn red_color(frame: &[u8], width: i32, x: i32, y: i32) -> bool {
    let i = idx(width, x, y);
    let (h, s, v) = hsv_like(frame[i + 2], frame[i + 1], frame[i]);
    ((0..=20).contains(&h) || (150..=179).contains(&h)) && s >= 45 && v >= 35
}

fn find_cart_center(frame: &[u8], width: i32, height: i32) -> Option<(i32, i32)> {
    let y0 = (height as f32 * 0.79) as i32;
    let y1 = (height as f32 * 0.91) as i32;
    let margin = (width as f32 * 0.08) as i32;
    let mut col_counts = vec![0i32; width as usize];
    for y in y0..y1 {
        for x in margin..(width - margin) {
            if red_color(frame, width, x, y) {
                col_counts[x as usize] += 1;
            }
        }
    }
    let mut best: Option<(i32, i32, i32)> = None;
    let mut start: Option<i32> = None;
    for x in 0..width {
        let active = col_counts[x as usize] >= 3;
        match (active, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                let len = x - s;
                if len >= (width as f32 * 0.08) as i32 && len <= (width as f32 * 0.34) as i32 {
                    let sum: i32 = col_counts[s as usize..x as usize].iter().sum();
                    if best.map_or(true, |b| sum > b.2) {
                        best = Some((s, x, sum));
                    }
                }
                start = None;
            }
            _ => {}
        }
    }
    let (x0, x1, _) = best?;
    Some(((x0 + x1) / 2, (height as f32 * 0.88) as i32))
}

fn detect_boot_motion(
    frame: &[u8],
    gray: &[u8],
    prev_gray: Option<&[u8]>,
    width: i32,
    roi: Roi,
    boot_size: BootSize,
    max_scale: f32,
    source: &'static str,
    expected: Option<(f32, f32)>,
) -> Option<Detection> {
    let prev = prev_gray?;
    let max_w = (boot_size.w as f32 * max_scale) as i32;
    let max_h = (boot_size.h as f32 * max_scale) as i32;
    let mut visited = vec![false; (roi.w * roi.h) as usize];
    let mut best: Option<Detection> = None;
    let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];

    for yy in 0..roi.h {
        for xx in 0..roi.w {
            let local_i = (yy * roi.w + xx) as usize;
            if visited[local_i] {
                continue;
            }
            let x = roi.x + xx;
            let y = roi.y + yy;
            let gi = (y * width + x) as usize;
            let motion = (gray[gi] as i32 - prev[gi] as i32).abs() >= 18;
            if !motion || !boot_color(frame, width, x, y) {
                visited[local_i] = true;
                continue;
            }

            let mut stack = vec![(xx, yy)];
            visited[local_i] = true;
            let mut min_x = xx;
            let mut max_x = xx;
            let mut min_y = yy;
            let mut max_y = yy;
            let mut count = 0i32;

            while let Some((cx, cy)) = stack.pop() {
                count += 1;
                min_x = min_x.min(cx);
                max_x = max_x.max(cx);
                min_y = min_y.min(cy);
                max_y = max_y.max(cy);
                for (dx, dy) in dirs {
                    let nx = cx + dx;
                    let ny = cy + dy;
                    if nx < 0 || ny < 0 || nx >= roi.w || ny >= roi.h {
                        continue;
                    }
                    let ni = (ny * roi.w + nx) as usize;
                    if visited[ni] {
                        continue;
                    }
                    let px = roi.x + nx;
                    let py = roi.y + ny;
                    let pgi = (py * width + px) as usize;
                    let is_motion = (gray[pgi] as i32 - prev[pgi] as i32).abs() >= 18;
                    if is_motion && boot_color(frame, width, px, py) {
                        visited[ni] = true;
                        stack.push((nx, ny));
                    } else {
                        visited[ni] = true;
                    }
                }
            }

            let w = max_x - min_x + 1;
            let h = max_y - min_y + 1;
            if count < 35 || w < 8 || h < 8 || w > max_w || h > max_h {
                continue;
            }
            let aspect = w as f32 / h.max(1) as f32;
            if !(0.25..=3.2).contains(&aspect) {
                continue;
            }
            let fill = count as f32 / (w * h).max(1) as f32;
            let size_score = (count as f32 / 260.0).min(1.0);
            let shape_score = 1.0 - ((aspect - 0.85).abs() / 2.2).min(1.0);
            let cx_abs = roi.x + (min_x + max_x) / 2;
            let cy_abs = roi.y + (min_y + max_y) / 2;
            let distance_penalty = expected.map_or(0.0, |(ex, ey)| {
                let dx = cx_abs as f32 - ex;
                let dy = cy_abs as f32 - ey;
                let diag = ((roi.w * roi.w + roi.h * roi.h) as f32).sqrt().max(1.0);
                ((dx * dx + dy * dy).sqrt() / diag).min(1.0) * 0.55
            });
            let score = 0.40 * fill + 0.45 * size_score + 0.15 * shape_score - distance_penalty;
            if best.map_or(true, |b| score > b.score) {
                best = Some(Detection {
                    x: cx_abs,
                    y: cy_abs,
                    score,
                    bbox: Roi { x: roi.x + min_x, y: roi.y + min_y, w, h },
                    source,
                });
            }
        }
    }
    best
}

fn simulate_motion(mut s: MotionState, target_t: f64, width: i32, height: i32, args: &Args) -> MotionState {
    let left = width as f32 * args.search_margin + args.boot_radius as f32;
    let right = width as f32 * (1.0 - args.search_margin) - args.boot_radius as f32;
    let top = height as f32 * args.search_top + args.boot_radius as f32;
    let bottom = height as f32 * args.search_bottom - args.boot_radius as f32;
    let dt_total = (target_t - s.last_t).max(0.0) as f32;
    let step = args.physics_step.clamp(0.004, 0.05);
    let steps = (dt_total / step).ceil().max(1.0) as i32;
    let dt = if steps > 0 { dt_total / steps as f32 } else { 0.0 };
    for _ in 0..steps {
        s.x += s.vx * dt;
        s.y += s.vy * dt;
        if s.x < left {
            s.x = left + (left - s.x);
            s.vx = s.vx.abs();
        } else if s.x > right {
            s.x = right - (s.x - right);
            s.vx = -s.vx.abs();
        }
        if s.y < top {
            s.y = top + (top - s.y);
            s.vy = s.vy.abs();
        } else if s.y > bottom {
            s.y = bottom - (s.y - bottom);
            s.vy = -s.vy.abs();
        }
        s.x = clamp_f32(s.x, left, right);
        s.y = clamp_f32(s.y, top, bottom);
    }
    s.last_t = target_t;
    s
}

fn trajectory_rois(pred: MotionState, width: i32, height: i32, base: Roi, boot_size: BootSize, args: &Args) -> Vec<Roi> {
    let base_radius = (((boot_size.w * boot_size.w + boot_size.h * boot_size.h) as f32).sqrt() / 2.0).ceil() as i32 + args.roi_padding;
    let radius = (base_radius as f32 + pred.speed() * args.roi_speed + pred.lost_frames as f32 * args.lost_roi_grow as f32) as i32;
    let horizon = (0.18 + pred.lost_frames as f32 * 0.08).clamp(0.18, args.intercept_horizon.min(1.0));
    let step = (args.physics_step * 2.0).clamp(0.025, 0.08);
    let mut rois = Vec::new();
    let mut sim = pred;
    let now = pred.last_t;
    let mut t = 0.0f32;
    rois.push(roi_around(sim.x, sim.y, radius, width, height, base));
    while t < horizon {
        t += step;
        sim = simulate_motion(sim, now + step as f64, width, height, args);
        sim.last_t = now;
        rois.push(roi_around(sim.x, sim.y, radius, width, height, base));
    }
    rois
}

fn update_boot(state: &mut Option<MotionState>, prev_det: &mut Option<(f64, f32, f32)>, det: Detection, now: f64) {
    let (vx, vy, seen) = if let Some((pt, px, py)) = *prev_det {
        let dt = (now - pt).max(1e-4) as f32;
        let raw_vx = (det.x as f32 - px) / dt;
        let raw_vy = (det.y as f32 - py) / dt;
        if let Some(s) = *state {
            let alpha = 0.45;
            (s.vx * (1.0 - alpha) + raw_vx * alpha, s.vy * (1.0 - alpha) + raw_vy * alpha, s.seen_frames + 1)
        } else {
            (raw_vx, raw_vy, 1)
        }
    } else {
        (0.0, 0.0, 1)
    };
    *state = Some(MotionState {
        x: det.x as f32,
        y: det.y as f32,
        vx,
        vy,
        last_t: now,
        lost_frames: 0,
        seen_frames: seen,
        score: det.score,
    });
    *prev_det = Some((now, det.x as f32, det.y as f32));
}

fn mark_lost(state: &mut Option<MotionState>, now: f64, width: i32, height: i32, args: &Args) {
    if let Some(s) = *state {
        let mut pred = simulate_motion(s, now, width, height, args);
        pred.lost_frames = s.lost_frames + 1;
        pred.score *= 0.92;
        *state = Some(pred);
    }
}

fn accept_boot_detection(state: Option<MotionState>, det: Detection, now: f64) -> bool {
    let Some(s) = state else {
        return true;
    };
    if s.seen_frames < 2 || s.lost_frames > 6 {
        return true;
    }
    let dt = (now - s.last_t).max(0.0) as f32;
    let expected_x = s.x + s.vx * dt;
    let expected_y = s.y + s.vy * dt;
    let dx = det.x as f32 - expected_x;
    let dy = det.y as f32 - expected_y;
    let dist = (dx * dx + dy * dy).sqrt();
    let gate = 150.0 + s.speed() * 0.12 + s.lost_frames as f32 * 45.0;
    dist <= gate || det.score >= 0.82
}

fn update_cart(state: &mut Option<MotionState>, det: Option<(i32, i32)>, now: f64, width: i32, height: i32, key: HeldKey, args: &Args) -> MotionState {
    let fallback_y = height as f32 * args.cart_y;
    match (det, *state) {
        (Some((x, y)), Some(s)) => {
            let dt = (now - s.last_t).max(1e-4) as f32;
            let raw_vx = (x as f32 - s.x) / dt;
            let vx = s.vx * 0.5 + raw_vx * 0.5;
            let next = MotionState { x: x as f32, y: y as f32, vx, vy: 0.0, last_t: now, lost_frames: 0, seen_frames: s.seen_frames + 1, score: 1.0 };
            *state = Some(next);
            next
        }
        (Some((x, y)), None) => {
            let next = MotionState { x: x as f32, y: y as f32, vx: 0.0, vy: 0.0, last_t: now, lost_frames: 0, seen_frames: 1, score: 1.0 };
            *state = Some(next);
            next
        }
        (None, Some(s)) => {
            let dt = (now - s.last_t).max(0.0) as f32;
            let vx = match key {
                HeldKey::A => -args.cart_speed.abs(),
                HeldKey::D => args.cart_speed.abs(),
                HeldKey::None => s.vx * 0.82,
            };
            let x = clamp_f32(s.x + vx * dt, 0.0, width as f32);
            let next = MotionState { x, y: s.y, vx, vy: 0.0, last_t: now, lost_frames: s.lost_frames + 1, seen_frames: s.seen_frames, score: s.score * 0.95 };
            *state = Some(next);
            next
        }
        (None, None) => {
            let next = MotionState { x: width as f32 / 2.0, y: fallback_y, vx: 0.0, vy: 0.0, last_t: now, lost_frames: 1, seen_frames: 0, score: 0.0 };
            *state = Some(next);
            next
        }
    }
}

fn key_event(vk: Word, up: bool) {
    let input = Input {
        r#type: INPUT_KEYBOARD,
        _pad: 0,
        ki: KeyBdInput {
            w_vk: vk,
            w_scan: 0,
            dw_flags: if up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dw_extra_info: 0,
        },
        _union_pad: 0,
    };
    unsafe {
        let sent = SendInput(1, &input, std::mem::size_of::<Input>() as i32);
        if sent == 0 {
            eprintln!("SendInput failed for vk={vk}");
        }
    }
}

fn click_game(rect: Rect) {
    let x = rect.left + rect.width / 2;
    let y = rect.top + (rect.height as f32 * 0.82) as i32;
    unsafe {
        SetCursorPos(x, y);
        sleep(Duration::from_millis(40));
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        sleep(Duration::from_millis(30));
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
    sleep(Duration::from_millis(120));
}

fn release_keys(held: &mut HeldKey, force_all: bool) {
    match *held {
        HeldKey::A => key_event(VK_A, true),
        HeldKey::D => key_event(VK_D, true),
        HeldKey::None if force_all => {
            key_event(VK_A, true);
            key_event(VK_D, true);
        }
        HeldKey::None => {}
    }
    *held = HeldKey::None;
}

fn hold_key(held: &mut HeldKey, key: HeldKey) {
    if *held == key {
        return;
    }
    release_keys(held, false);
    match key {
        HeldKey::A => key_event(VK_A, false),
        HeldKey::D => key_event(VK_D, false),
        HeldKey::None => {}
    }
    *held = key;
}

fn steer_to(target_x: f32, cart: MotionState, intercept_t: f32, held: &mut HeldKey, args: &Args) -> (char, f32) {
    let lookahead = (intercept_t * 0.35).clamp(0.0, 0.18);
    let error = target_x - (cart.x + cart.vx * lookahead);
    let switch_zone = args.deadzone * 1.45;
    let release_zone = args.deadzone * 0.55;
    match *held {
        HeldKey::D => {
            if error < -switch_zone {
                hold_key(held, HeldKey::A);
                ('A', error)
            } else if error > release_zone {
                ('D', error)
            } else {
                release_keys(held, false);
                ('-', error)
            }
        }
        HeldKey::A => {
            if error > switch_zone {
                hold_key(held, HeldKey::D);
                ('D', error)
            } else if error < -release_zone {
                ('A', error)
            } else {
                release_keys(held, false);
                ('-', error)
            }
        }
        HeldKey::None => {
            if error > args.deadzone {
                hold_key(held, HeldKey::D);
                ('D', error)
            } else if error < -args.deadzone {
                hold_key(held, HeldKey::A);
                ('A', error)
            } else {
                ('-', error)
            }
        }
    }
}

fn run_startup() {
    key_event(VK_D, false);
    sleep(Duration::from_millis(600));
    key_event(VK_D, true);
    for _ in 0..2 {
        key_event(VK_SPACE, false);
        sleep(Duration::from_millis(250));
        key_event(VK_SPACE, true);
        sleep(Duration::from_millis(180));
    }
    key_event(VK_A, false);
    sleep(Duration::from_millis(600));
    key_event(VK_A, true);
    for _ in 0..2 {
        key_event(VK_SPACE, false);
        sleep(Duration::from_millis(250));
        key_event(VK_SPACE, true);
        sleep(Duration::from_millis(180));
    }
}

fn tap_space() {
    key_event(VK_SPACE, false);
    sleep(Duration::from_millis(70));
    key_event(VK_SPACE, true);
}

fn open_log(path: &str) -> io::Result<Option<BufWriter<File>>> {
    if path.is_empty() {
        return Ok(None);
    }
    let mut w = BufWriter::new(File::create(path)?);
    writeln!(
        w,
        "frame\ttime\tboot_det_x\tboot_det_y\tboot_score\tboot_source\tboot_box\tboot_track_x\tboot_track_y\tboot_vx\tboot_vy\tboot_lost\tcart_det_x\tcart_det_y\tcart_track_x\tcart_track_y\tcart_vx\tcart_lost\ttarget_x\terror\taction\tkey\treject_reason\tloop_ms\tfps\tboot_expected_w\tboot_expected_h\tsearch_roi_count\tsearch_roi_area\tsearch_rois\tgrab_ms\tgray_ms\tcart_ms\tboot_search_ms\tintercept_ms\tcontrol_ms\tlog_ms"
    )?;
    Ok(Some(w))
}

fn main() -> io::Result<()> {
    if !cfg!(target_os = "windows") {
        eprintln!("boot_catcher_rs is Windows-only");
        return Ok(());
    }

    let args = parse_args();
    unsafe {
        SetConsoleCtrlHandler(Some(console_ctrl_handler), 1);
    }
    if args.debug {
        println!("Debug is console-only in Rust build. INPUT size={}", std::mem::size_of::<Input>());
    }
    let boot_size = read_png_size(&args.template).unwrap_or(BootSize { w: 88, h: 112 });
    println!("Template size: {}x{}", boot_size.w, boot_size.h);
    let rect = calibrate_game_rect()?;
    println!("Start in 3 seconds. Focus the game.");
    sleep(Duration::from_secs(3));
    click_game(rect);
    if !args.no_startup {
        println!("Sending startup keys...");
        run_startup();
    }

    let mut cap = ScreenCapture::new(rect.width, rect.height)?;
    let mut gray = Vec::new();
    let mut prev_gray: Option<Vec<u8>> = None;
    let mut boot_state: Option<MotionState> = None;
    let mut prev_boot_det: Option<(f64, f32, f32)> = None;
    let mut last_found_boot_x: Option<f32> = None;
    let mut cart_state: Option<MotionState> = None;
    let mut held = HeldKey::None;
    let mut log = open_log(&args.log_file)?;
    let mut boot_missing_since: Option<f64> = None;
    let mut rescue_space_remaining = 0;
    let mut next_rescue_space_at = f64::INFINITY;
    let frame_delay = Duration::from_secs_f64(1.0 / args.fps.max(1.0));
    let start = Instant::now();
    let mut fps_est = 0.0f64;
    let mut frame_index = 0u64;
    let control_enabled = true;

    while !STOP.load(Ordering::SeqCst) {
        let loop_t0 = Instant::now();
        let now = start.elapsed().as_secs_f64();
        let t = Instant::now();
        let frame = cap.grab_bgra(rect)?;
        let grab_ms = t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        bgra_to_gray(frame, rect.width, rect.height, &mut gray);
        let gray_ms = t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let cart_det = find_cart_center(frame, rect.width, rect.height);
        let cart = update_cart(&mut cart_state, cart_det, now, rect.width, rect.height, held, &args);
        let cart_ms = t.elapsed().as_secs_f64() * 1000.0;

        let base_roi = base_search_roi(rect.width, rect.height, &args);
        let predicted = boot_state.map(|s| simulate_motion(s, now, rect.width, rect.height, &args));

        let t = Instant::now();
        let mut search_rois = Vec::new();
        let mut boot_det = None;
        if let Some(pred) = predicted {
            search_rois = trajectory_rois(pred, rect.width, rect.height, base_roi, boot_size, &args);
            for (i, roi) in search_rois.iter().copied().enumerate() {
                let src = if i == 0 { "traj0-motion" } else { "traj-motion" };
                boot_det = detect_boot_motion(
                    frame,
                    &gray,
                    prev_gray.as_deref(),
                    rect.width,
                    roi,
                    boot_size,
                    args.boot_size_max_scale,
                    src,
                    Some((pred.x, pred.y)),
                );
                if boot_det.is_some() {
                    break;
                }
            }
        } else {
            search_rois.push(base_roi);
            boot_det = detect_boot_motion(
                frame,
                &gray,
                prev_gray.as_deref(),
                rect.width,
                base_roi,
                boot_size,
                args.boot_size_max_scale,
                "full-init-motion",
                None,
            );
        }
        let boot_search_ms = t.elapsed().as_secs_f64() * 1000.0;

        let mut reject_reason = String::new();
        if let Some(det) = boot_det {
            let cart_line_y = cart_det.map_or(rect.height as f32 * args.cart_y, |(_, y)| y as f32);
            if det.y as f32 > cart_line_y + args.boot_radius as f32 {
                reject_reason = format!("boot_below_cart det_y={} cart_y={cart_line_y:.0}", det.y);
                boot_det = None;
            }
        }

        if let Some(det) = boot_det {
            if accept_boot_detection(boot_state, det, now) {
                last_found_boot_x = Some(det.x as f32);
                update_boot(&mut boot_state, &mut prev_boot_det, det, now);
            } else {
                reject_reason = "tracker_gate".to_string();
                boot_det = None;
                mark_lost(&mut boot_state, now, rect.width, rect.height, &args);
            }
        } else {
            mark_lost(&mut boot_state, now, rect.width, rect.height, &args);
            if boot_state.map_or(false, |s| s.lost_frames > args.lost_keep_frames) {
                boot_state = None;
                prev_boot_det = None;
                last_found_boot_x = None;
            }
        }

        if boot_state.is_some() {
            boot_missing_since = None;
            rescue_space_remaining = 0;
            next_rescue_space_at = f64::INFINITY;
        } else if boot_missing_since.is_none() {
            boot_missing_since = Some(now);
            next_rescue_space_at = now + args.rescue_space_delay.max(0.0);
        }

        let t = Instant::now();
        let target_x = last_found_boot_x;
        let intercept_t = 0.0;
        let intercept_ms = t.elapsed().as_secs_f64() * 1000.0;

        let t = Instant::now();
        let mut action = '-';
        let mut error = None;
        if let Some(boot) = boot_state {
            if !control_enabled {
                release_keys(&mut held, false);
                reject_reason = if reject_reason.is_empty() { "control_disabled".to_string() } else { reject_reason };
            } else if boot.lost_frames <= args.lost_keep_frames && target_x.is_some() {
                let (a, e) = steer_to(target_x.unwrap(), cart, intercept_t, &mut held, &args);
                action = a;
                error = Some(e);
            } else {
                release_keys(&mut held, false);
            }
        } else {
            release_keys(&mut held, false);
            if control_enabled && !args.no_rescue_space && args.rescue_space_taps > 0 && now >= next_rescue_space_at {
                tap_space();
                action = 'S';
                reject_reason = if reject_reason.is_empty() { "rescue_space".to_string() } else { reject_reason };
                if rescue_space_remaining <= 0 {
                    rescue_space_remaining = args.rescue_space_taps - 1;
                } else {
                    rescue_space_remaining -= 1;
                }
                next_rescue_space_at = if rescue_space_remaining > 0 {
                    now + args.rescue_space_interval.max(0.05)
                } else {
                    now + args.rescue_space_cooldown.max(args.rescue_space_interval.max(0.05))
                };
            }
        }
        let control_ms = t.elapsed().as_secs_f64() * 1000.0;

        let elapsed = loop_t0.elapsed();
        let fps_now = 1.0 / elapsed.as_secs_f64().max(1e-6);
        fps_est = if fps_est <= 0.0 { fps_now } else { fps_est * 0.85 + fps_now * 0.15 };

        if let Some(w) = log.as_mut() {
            let t = Instant::now();
            let roi_text = search_rois
                .iter()
                .map(|r| format!("{},{},{},{}", r.x, r.y, r.w, r.h))
                .collect::<Vec<_>>()
                .join(";");
            let roi_area: i32 = search_rois.iter().map(|r| r.w * r.h).sum();
            let bd = boot_det;
            let bs = boot_state;
            let row = vec![
                frame_index.to_string(),
                format!("{now:.6}"),
                bd.map_or(String::new(), |d| d.x.to_string()),
                bd.map_or(String::new(), |d| d.y.to_string()),
                bd.map_or(String::new(), |d| format!("{:.3}", d.score)),
                bd.map_or(String::new(), |d| d.source.to_string()),
                bd.map_or(String::new(), |d| format!("{},{},{},{}", d.bbox.x, d.bbox.y, d.bbox.w, d.bbox.h)),
                bs.map_or(String::new(), |s| format!("{:.1}", s.x)),
                bs.map_or(String::new(), |s| format!("{:.1}", s.y)),
                bs.map_or(String::new(), |s| format!("{:.1}", s.vx)),
                bs.map_or(String::new(), |s| format!("{:.1}", s.vy)),
                bs.map_or(String::new(), |s| s.lost_frames.to_string()),
                cart_det.map_or(String::new(), |(x, _)| x.to_string()),
                cart_det.map_or(String::new(), |(_, y)| y.to_string()),
                format!("{:.1}", cart.x),
                format!("{:.1}", cart.y),
                format!("{:.1}", cart.vx),
                cart.lost_frames.to_string(),
                target_x.map_or(String::new(), |v| format!("{v:.1}")),
                error.map_or(String::new(), |v| format!("{v:.1}")),
                action.to_string(),
                format!("{held:?}"),
                reject_reason.clone(),
                format!("{:.2}", elapsed.as_secs_f64() * 1000.0),
                format!("{fps_est:.2}"),
                boot_size.w.to_string(),
                boot_size.h.to_string(),
                search_rois.len().to_string(),
                roi_area.to_string(),
                roi_text,
                format!("{grab_ms:.2}"),
                format!("{gray_ms:.2}"),
                format!("{cart_ms:.2}"),
                format!("{boot_search_ms:.2}"),
                format!("{intercept_ms:.2}"),
                format!("{control_ms:.2}"),
                format!("{:.2}", t.elapsed().as_secs_f64() * 1000.0),
            ];
            writeln!(w, "{}", row.join("\t"))?;
            if frame_index % 30 == 0 {
                w.flush()?;
            }
        }

        if args.debug && frame_index % 10 == 0 {
            println!(
                "frame={frame_index} fps={fps_est:.1} action={action} key={held:?} boot={:?} reject={}",
                boot_state.map(|s| (s.x as i32, s.y as i32, s.lost_frames)),
                reject_reason
            );
        }

        prev_gray = Some(gray.clone());
        frame_index += 1;
        if elapsed < frame_delay {
            sleep(frame_delay - elapsed);
        }

    }
    release_keys(&mut held, true);
    if let Some(w) = log.as_mut() {
        w.flush()?;
    }
    println!("Stopped.");
    Ok(())
}
