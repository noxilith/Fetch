//! The 3D side: logo -> point cloud, terminal layout, z-buffered rasterizing
//! with Blinn-Phong shading, and composing a frame of text.

use crate::config::{HAlign, VAlign};
use crate::logo::{char_weight, Logo};
use crate::shading::{Shading, MAX_SUB_COLS, MAX_SUB_ROWS, QUADRANT_GLYPHS};
use crate::util::{emit_clipped, visible_width};

pub const ANIM_WIDTH: usize = 60;
pub const MAX_HEIGHT: usize = 200;
pub const GAP: usize = 2;

// Headroom for the sub-cell modes, which sample a finer grid and so need more
// points to fill it: sextants at --size 3 land near 150k.
const MAX_POINTS: usize = 200_000;

pub const K2: f32 = 5.5;

// --- point cloud ---

#[derive(Default)]
pub struct Points {
    pub px: Vec<f32>,
    pub py: Vec<f32>,
    pub pz: Vec<f32>,
    pub nx: Vec<f32>,
    pub ny: Vec<f32>,
    pub nz: Vec<f32>,
    pub color: Vec<i32>,
}

impl Points {
    pub fn len(&self) -> usize {
        self.px.len()
    }

    fn shrink_to_fit(&mut self) {
        for v in [&mut self.px, &mut self.py, &mut self.pz, &mut self.nx, &mut self.ny, &mut self.nz] {
            v.shrink_to_fit();
        }
        self.color.shrink_to_fit();
    }

    fn push(&mut self, p: [f32; 3], n: [f32; 3], color: i32) {
        self.px.push(p[0]);
        self.py.push(p[1]);
        self.pz.push(p[2]);
        self.nx.push(n[0]);
        self.ny.push(n[1]);
        self.nz.push(n[2]);
        self.color.push(color);
    }
}

/// Turn each logo character into a column of points whose height follows its
/// visual density, with a surface normal for shading. `depth` is the user's
/// depth scale; when `depth_user_set` is false it is auto-boosted for flat logos.
pub fn build_points(
    logo: &Logo,
    size_scale: f32,
    mut depth: f32,
    depth_user_set: bool,
    sub_rows: usize,
) -> Points {
    let sx = 0.07f32;
    let sy = 0.14f32;
    let rows = logo.cells.len();
    let cols = logo.cols;
    let mut pts = Points::default();
    if rows == 0 || cols == 0 {
        return pts;
    }
    let cx = (cols as f32 - 1.0) * 0.5;
    let cy = (rows as f32 - 1.0) * 0.5;
    let z_layers = ((6.0 * size_scale) as usize).max(6);

    let mut hmap = vec![0.0f32; rows * cols];
    for r in 0..rows {
        for c in 0..logo.cells[r].len().min(cols) {
            hmap[r * cols + c] = char_weight(logo.cells[r][c].ch);
        }
    }
    let h = |r: usize, c: usize| hmap[r * cols + c];

    // Logos with low height variance look flat: boost depth to compensate.
    if !depth_user_set {
        let (mut sum, mut sum2, mut n) = (0.0f32, 0.0f32, 0usize);
        for &v in &hmap {
            if v > 0.0 {
                sum += v;
                sum2 += v * v;
                n += 1;
            }
        }
        if n > 0 {
            let mean = sum / n as f32;
            let variance = sum2 / n as f32 - mean * mean;
            let stddev = (if variance > 0.0 { variance } else { 0.0 }).sqrt();
            // stddev ranges ~0.05 (flat) to ~0.3 (high contrast): flat logos
            // get up to a 3x depth boost.
            if stddev < 0.25 {
                depth *= 1.0 + 2.0 * (0.25 - stddev) / 0.25;
            }
        }
    }
    let zmax = 0.18f32 * depth;

    // Per-cell face normals from the height gradient.
    let mut gn = vec![[0.0f32, 0.0, 1.0]; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            if h(r, c) <= 0.0 {
                continue;
            }
            let mut dhdx = 0.0f32;
            let mut dhdy = 0.0f32;
            if c > 0 && c < cols - 1 {
                dhdx = (h(r, c + 1) - h(r, c - 1)) * 0.5;
            } else if c == 0 && cols > 1 {
                dhdx = h(r, c + 1) - h(r, c);
            } else if c > 0 {
                dhdx = h(r, c) - h(r, c - 1);
            }
            if r > 0 && r < rows - 1 {
                dhdy = (h(r + 1, c) - h(r - 1, c)) * 0.5;
            } else if r == 0 && rows > 1 {
                dhdy = h(r + 1, c) - h(r, c);
            } else if r > 0 {
                dhdy = h(r, c) - h(r - 1, c);
            }
            dhdx /= sx;
            dhdy /= sy;
            let (nnx, nny, nnz) = (-dhdx, dhdy, 1.0f32);
            let l = (nnx * nnx + nny * nny + nnz * nnz).sqrt();
            gn[r * cols + c] = [nnx / l, nny / l, nnz / l];
        }
    }

    // Subdivide the grid for larger sizes to avoid gaps; sub-cell modes sample
    // a finer grid, so they need proportionally more points.
    let subdiv = ((size_scale * sub_rows as f32) as usize).max(sub_rows);

    let neighbor_h = |row: usize, col: usize, dr: i32, dc: i32| -> f32 {
        let nr = row as i32 + dr;
        let nc = col as i32 + dc;
        if nr >= 0 && (nr as usize) < rows && nc >= 0 && (nc as usize) < cols {
            h(nr as usize, nc as usize)
        } else {
            0.0
        }
    };

    'outer: for row in 0..rows {
        for col in 0..cols {
            let hh = h(row, col);
            if hh <= 0.0 {
                continue;
            }
            // Only interior cells get side layers; edge cells (next to empty
            // space) get just front + back, avoiding "tail" artifacts.
            let mut is_edge = false;
            for dr in -1..=1 {
                for dc in -1..=1 {
                    if (dr != 0 || dc != 0) && neighbor_h(row, col, dr, dc) <= 0.0 {
                        is_edge = true;
                    }
                }
            }

            for sr in 0..subdiv {
                for sc in 0..subdiv {
                    let frow = row as f32 + sr as f32 / subdiv as f32;
                    let fcol = col as f32 + sc as f32 / subdiv as f32;

                    let mut ih = hh;
                    if sr > 0 || sc > 0 {
                        let fr = sr as f32 / subdiv as f32;
                        let fc = sc as f32 / subdiv as f32;
                        let nr = (row + usize::from(sr > 0)).min(rows - 1);
                        let nc = (col + usize::from(sc > 0)).min(cols - 1);
                        let h00 = h(row, col);
                        let h10 = h(nr, col);
                        let h01 = h(row, nc);
                        let h11 = h(nr, nc);
                        ih = h00 * (1.0 - fr) * (1.0 - fc)
                            + h10 * fr * (1.0 - fc)
                            + h01 * (1.0 - fr) * fc
                            + h11 * fr * fc;
                        if ih <= 0.0 {
                            continue;
                        }
                    }

                    let ox = (fcol - cx) * sx;
                    let oy = (cy - frow) * sy;
                    let zr = ih * zmax;
                    let layers = if is_edge || ih < 0.15 { 2 } else { z_layers };
                    let g = gn[row * cols + col];

                    for k in 0..layers {
                        if pts.len() >= MAX_POINTS {
                            break;
                        }
                        let t = (k as f32 / (layers - 1) as f32) - 0.5;
                        let pz = t * 2.0 * zr;
                        // Uncolored logos are two-toned by surface: front and
                        // back faces take the inner color, sides the outer.
                        let color = if logo.has_ansi {
                            logo.cells[row].get(col).map_or(0, |c| c.color)
                        } else {
                            i32::from(k == 0 || k == layers - 1)
                        };
                        let n = if k == 0 {
                            [g[0], g[1], -g[2]]
                        } else if k == layers - 1 {
                            [g[0], g[1], g[2]]
                        } else {
                            let (mut ex, mut ey) = (0.0f32, 0.0f32);
                            for dr in -1..=1 {
                                for dc in -1..=1 {
                                    if (dr != 0 || dc != 0) && neighbor_h(row, col, dr, dc) < hh {
                                        ex += dc as f32;
                                        ey += (-dr) as f32;
                                    }
                                }
                            }
                            let el = (ex * ex + ey * ey).sqrt();
                            if el > 1e-6 {
                                ex /= el;
                                ey /= el;
                            }
                            let tn = (k as f32 / (layers - 1) as f32) * 2.0 - 1.0;
                            let side = (1.0 - tn * tn).sqrt();
                            [ex * side, ey * side, tn]
                        };
                        pts.push([ox, oy, pz], n, color);
                    }
                    if pts.len() >= MAX_POINTS {
                        break 'outer;
                    }
                }
            }
        }
    }
    pts.shrink_to_fit();
    pts
}

// --- layout ---

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Layout {
    pub render_height: i32,
    /// Rows the logo may span (<= render_height).
    pub logo_height: i32,
    /// Canvas columns; shrinks on narrow terminals.
    pub anim_width: i32,
    /// Info below the logo instead of beside it.
    pub stacked: bool,
    /// Clip info lines to this many visible columns (-1 = no clip).
    pub info_clip_cols: i32,
    pub stacked_info_rows: i32,
}

/// Sizing + layout, shared by startup and SIGWINCH so a resize reproduces the
/// startup look. Wide terminals get the full 60-col canvas beside the info;
/// medium ones shrink the canvas to keep the info intact; phone-width ones
/// stack the info below the logo. Info lines clip instead of wrapping.
pub fn apply_layout(
    term_rows: i32,
    term_cols: i32,
    size_scale: f32,
    config_height: i64,
    show_info: bool,
    info: &[String],
) -> Layout {
    let mut rows = term_rows;
    let cols = term_cols;
    if rows > 1 {
        rows -= 1; // leave 1 row margin to prevent scroll-jitter
    }

    // Ideal height: config/flag override > auto-fit to info lines > default
    let mut ideal = (36.0 * size_scale) as i32;
    if config_height > 0 {
        ideal = config_height as i32;
    } else if show_info && !info.is_empty() {
        ideal = ideal.max(info.len() as i32 + 2);
    }
    let ideal = ideal.clamp(20, MAX_HEIGHT as i32);

    let mut l = Layout {
        render_height: ideal,
        logo_height: ideal,
        anim_width: ANIM_WIDTH as i32,
        stacked: false,
        info_clip_cols: -1,
        stacked_info_rows: 0,
    };

    if cols <= 0 {
        // no width info (not a tty): cap height only
        if rows > 0 && l.render_height > rows {
            l.render_height = rows;
        }
        l.logo_height = l.render_height;
        return l;
    }

    let info_cols = if show_info {
        info.iter().map(|s| visible_width(s) as i32).max().unwrap_or(0)
    } else {
        0
    };
    let aw = ANIM_WIDTH as i32;
    let gap = GAP as i32;

    if show_info && cols < aw + gap + info_cols {
        // Full canvas + info doesn't fit: give the info what it needs and
        // shrink the canvas, unless that leaves it unreadable; then stack the
        // info below a full-width logo instead.
        l.anim_width = cols - gap - info_cols;
        if l.anim_width < 20 {
            l.stacked = true;
            l.anim_width = cols.min(aw);
        }
    } else if l.anim_width > cols {
        l.anim_width = cols;
    }

    // The projection needs ~5/3 the columns of its rows: shrink the logo on
    // narrow canvases instead of clipping its sides. Only the logo shrinks;
    // render_height keeps covering the info lines so none are dropped.
    let wcap = l.anim_width * 3 / 5;

    if l.stacked {
        if l.render_height > wcap {
            l.render_height = wcap;
        }
        l.stacked_info_rows = info.len() as i32;
        if rows > 0 {
            let budget = rows - info.len() as i32 - 1; // 1 spacer row
            if budget < l.render_height {
                l.render_height = budget;
            }
            if l.render_height < 6 {
                l.render_height = 0; // too small to read: show info only
            }
            let info_budget = rows - if l.render_height > 0 { l.render_height + 1 } else { 0 };
            if l.stacked_info_rows > info_budget {
                l.stacked_info_rows = info_budget.max(0);
            }
        }
        l.logo_height = l.render_height;
        l.info_clip_cols = cols;
    } else {
        if show_info {
            l.info_clip_cols = cols - l.anim_width - gap;
        }
        if rows > 0 && l.render_height > rows {
            l.render_height = rows;
        }
        l.logo_height = l.render_height.min(wcap);
        if !show_info {
            l.render_height = l.logo_height; // no info: no point in blank rows
        }
    }
    l
}

/// Extra (vertical rows, horizontal columns) padding from the alignment config.
pub fn alignment_padding(
    v: VAlign,
    h: HAlign,
    term_rows: i32,
    term_cols: i32,
    layout: &Layout,
    info: &[String],
) -> (i32, i32) {
    let max = info.iter().map(|s| visible_width(s) as i32).max().unwrap_or(0);
    let vertical = match v {
        VAlign::Top => 0,
        VAlign::Center => term_rows / 2 - layout.render_height / 2,
        VAlign::Bottom => term_rows - layout.render_height,
    };
    let width = layout.anim_width + GAP as i32 + max;
    let horizontal = match h {
        HAlign::Left => 0,
        HAlign::Center => term_cols / 2 - width / 2,
        HAlign::Right => term_cols - width,
    };
    (vertical, horizontal)
}

// --- rasterizing ---

const SUB_H: usize = MAX_HEIGHT * MAX_SUB_ROWS;
const SUB_W: usize = ANIM_WIDTH * MAX_SUB_COLS;

pub struct Camera {
    pub k1: f32,
    pub y_center: f32,
    pub light: [f32; 3],
}

/// Render buffers, one entry per sub-cell: z-buffer (0 = empty), luminance, color.
pub struct Raster {
    zbuf: Vec<f32>,
    lumbuf: Vec<f32>,
    colorbuf: Vec<i32>,
}

impl Raster {
    pub fn new() -> Self {
        Raster {
            zbuf: vec![0.0; SUB_H * SUB_W],
            lumbuf: vec![0.0; SUB_H * SUB_W],
            colorbuf: vec![0; SUB_H * SUB_W],
        }
    }

    fn clear(&mut self, render_height: i32, sub_rows: usize) {
        let n = (render_height.max(0) as usize * sub_rows * SUB_W).min(SUB_H * SUB_W);
        self.zbuf[..n].fill(0.0);
        self.lumbuf[..n].fill(0.0);
        self.colorbuf[..n].fill(0);
    }

    /// Rotate the cloud by (a, b), project it and shade it into the buffers.
    pub fn draw(
        &mut self,
        pts: &Points,
        a: f32,
        b: f32,
        cam: &Camera,
        layout: &Layout,
        shading: &Shading,
    ) {
        self.clear(layout.render_height, shading.sub_rows);
        let (ca, sa) = (a.cos(), a.sin());
        let (cb, sb) = (b.cos(), b.sin());
        let [lx, ly, lz] = cam.light;

        // Blinn-Phong half-vector (the view direction is constant (0,0,-1))
        let hx0 = lx + 0.0;
        let hy0 = ly + 0.0;
        let hz0 = lz - 1.0;
        let hl0 = (hx0 * hx0 + hy0 * hy0 + hz0 * hz0).sqrt();
        let (hlx, hly, hlz) = (hx0 / hl0, hy0 / hl0, hz0 / hl0);

        let half_aw = layout.anim_width as f32 * 0.5;
        let k1x2 = cam.k1 * 2.0;
        let sub_cols = shading.sub_cols as i32;
        let sub_rows = shading.sub_rows as i32;
        let max_x = layout.anim_width * sub_cols;
        let max_y = layout.render_height * sub_rows;

        let n = pts.len();
        let (pxs, pys, pzs) = (&pts.px[..n], &pts.py[..n], &pts.pz[..n]);
        let (nxs, nys, nzs) = (&pts.nx[..n], &pts.ny[..n], &pts.nz[..n]);
        let colors = &pts.color[..n];
        for i in 0..n {
            let (px, py, pz) = (pxs[i], pys[i], pzs[i]);

            let y1 = py * ca - pz * sa;
            let z1 = py * sa + pz * ca;
            let x2 = px * cb + z1 * sb;
            let z2 = -px * sb + z1 * cb;
            let y2 = y1;

            let zc = z2 + K2;
            if zc < 0.1 {
                continue;
            }
            let ooz = 1.0 / zc;
            let xs = ((half_aw + k1x2 * x2 * ooz) * sub_cols as f32) as i32;
            let ys = ((cam.y_center - cam.k1 * y2 * ooz) * sub_rows as f32) as i32;
            if xs < 0 || xs >= max_x || ys < 0 || ys >= max_y {
                continue;
            }
            let idx = ys as usize * SUB_W + xs as usize;

            if ooz > self.zbuf[idx] {
                // Only the points that win the z-test need their normal rotated.
                let (nx, ny, nz) = (nxs[i], nys[i], nzs[i]);
                let ny1 = ny * ca - nz * sa;
                let nz1 = ny * sa + nz * ca;
                let nx2 = nx * cb + nz1 * sb;
                let nz2 = -nx * sb + nz1 * cb;
                let ny2 = ny1;

                let mut diff = nx2 * lx + ny2 * ly + nz2 * lz;
                if diff < 0.0 {
                    diff = 0.0;
                }
                let mut spec_dot = nx2 * hlx + ny2 * hly + nz2 * hlz;
                if spec_dot < 0.0 {
                    spec_dot = 0.0;
                }
                let mut spec = spec_dot * spec_dot;
                spec *= spec;
                spec *= spec;

                let mut lum = 0.08 + 0.62 * diff + 0.30 * spec;
                if lum > 1.0 {
                    lum = 1.0;
                }
                self.zbuf[idx] = ooz;
                self.lumbuf[idx] = lum;
                self.colorbuf[idx] = colors[i];
            }
        }
    }

    /// Collapse one cell's sub-samples into a glyph, comparing the two ways the
    /// cell can be drawn against the ink it should carry: the sub-cell block is
    /// always full ink over the part it covers, the ramp spreads a lighter
    /// shade over the whole cell. Picking by ink keeps crisp edges where the
    /// surface is lit and stops dim edges from ringing the logo in a bright
    /// outline. `None` for an empty cell; otherwise the glyph and its color.
    fn cell_glyph<'a>(&self, row: usize, col: usize, shading: &'a Shading) -> Option<(&'a str, i32)> {
        let (sub_rows, sub_cols) = (shading.sub_rows, shading.sub_cols);
        let smax = shading.chars.len() as i32 - 1;
        let x0 = col * sub_cols;
        let y0 = row * sub_rows;
        let total = sub_rows * sub_cols;
        let mut mask = 0usize;
        let mut n = 0i32;
        let mut lsum = 0.0f32;
        let mut best = 0.0f32;
        let mut color = 0;
        for sr in 0..sub_rows {
            for sc in 0..sub_cols {
                let idx = (y0 + sr) * SUB_W + x0 + sc;
                let z = self.zbuf[idx];
                if z <= 0.0 {
                    continue;
                }
                mask |= 1 << (sr * sub_cols + sc);
                lsum += self.lumbuf[idx];
                n += 1;
                if z > best {
                    best = z;
                    color = self.colorbuf[idx];
                }
            }
        }
        if n == 0 {
            return None;
        }

        let coverage = n as f32 / total as f32;
        let ink = lsum / n as f32 * coverage;
        // Round to the nearest step: truncating biases every cell one level
        // lighter and leaves the top of the ramp unreachable.
        let ci = ((ink * smax as f32 + 0.5) as i32).clamp(0, smax);
        if mask != (1 << total) - 1
            && (coverage - ink).abs()
                <= ((ci as f32 + 1.0) / shading.chars.len() as f32 - ink).abs()
        {
            let g = if sub_rows == 3 {
                shading.sextants[mask].as_str()
            } else {
                QUADRANT_GLYPHS[mask]
            };
            return Some((g, color));
        }
        Some((shading.chars[ci as usize].as_str(), color))
    }
}

pub struct Colors<'a> {
    pub inner: &'a str,
    pub outer: &'a str,
    pub use_color: bool,
    pub logo_has_ansi: bool,
}

/// Compose a full frame: cursor home, the logo rows with the info beside (or
/// below, when stacked), each line cleared to end-of-line.
pub fn compose_frame(
    out: &mut Vec<u8>,
    raster: &Raster,
    shading: &Shading,
    layout: &Layout,
    colors: &Colors,
    info: &[String],
    show_info: bool,
    padding: (i32, i32),
) {
    const RESET: &[u8] = b"\x1b[0m";
    const CLEAR: &[u8] = b"\x1b[K";
    let (top_padding, left_padding) = padding;
    let fetch_start: i32 = i32::from(show_info);
    let aw = layout.anim_width.max(0) as usize;

    out.clear();
    out.extend_from_slice(b"\x1b[H");
    out.resize(out.len() + top_padding.max(0) as usize, b'\n');

    for i in 0..layout.render_height.max(0) {
        out.resize(out.len() + left_padding.max(0) as usize, b' ');

        if !colors.use_color {
            for j in 0..aw {
                match raster.cell_glyph(i as usize, j, shading) {
                    Some((g, _)) => out.extend_from_slice(g.as_bytes()),
                    None => out.push(b' '),
                }
            }
        } else {
            let mut prev_color = -1;
            for j in 0..aw {
                match raster.cell_glyph(i as usize, j, shading) {
                    None => {
                        if prev_color != -1 {
                            out.extend_from_slice(RESET);
                            prev_color = -1;
                        }
                        out.push(b' ');
                    }
                    Some((g, c)) => {
                        if c != prev_color {
                            if colors.logo_has_ansi && c > 0 && c < 128 {
                                push_sgr(out, c);
                            } else {
                                let cs = if c == 1 { colors.inner } else { colors.outer };
                                out.extend_from_slice(cs.as_bytes());
                            }
                            prev_color = c;
                        }
                        out.extend_from_slice(g.as_bytes());
                    }
                }
            }
            if prev_color != -1 {
                out.extend_from_slice(RESET);
            }
        }

        let fi = i - fetch_start;
        if !layout.stacked && fi >= 0 && (fi as usize) < info.len() {
            out.resize(out.len() + GAP, b' ');
            emit_clipped(out, info[fi as usize].as_bytes(), layout.info_clip_cols);
        }
        out.extend_from_slice(CLEAR);
        out.push(b'\n');
    }

    if layout.stacked && layout.stacked_info_rows > 0 {
        if layout.render_height > 0 {
            out.extend_from_slice(CLEAR);
            out.push(b'\n');
        }
        for line in info.iter().take(layout.stacked_info_rows as usize) {
            emit_clipped(out, line.as_bytes(), layout.info_clip_cols);
            out.extend_from_slice(CLEAR);
            out.push(b'\n');
        }
    }
}

/// `ESC [ 1 ; <c> m` for a color code below 128, without allocating.
fn push_sgr(out: &mut Vec<u8>, c: i32) {
    out.extend_from_slice(b"\x1b[1;");
    if c >= 100 {
        out.push(b'0' + (c / 100) as u8);
    }
    if c >= 10 {
        out.push(b'0' + (c / 10 % 10) as u8);
    }
    out.push(b'0' + (c % 10) as u8);
    out.push(b'm');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_logos_build_finite_points() {
        for shape in 0..3 {
            let raw: Vec<String> = match shape {
                0 => vec!["@".into()],
                1 => vec!["@@@".into()],
                _ => vec!["@".into(); 3],
            };
            let mut logo = Logo {
                raw,
                ..Default::default()
            };
            logo.process();
            let pts = build_points(&logo, 1.0, 1.0, true, 1);
            assert!(pts.len() > 0, "shape {shape}");
            for v in [&pts.px, &pts.py, &pts.pz, &pts.nx, &pts.ny, &pts.nz] {
                assert!(v.iter().all(|x| x.is_finite()), "shape {shape}");
            }
        }
    }

    #[test]
    fn layout_without_tty_caps_height_only() {
        let info = vec!["a".to_string(); 10];
        let l = apply_layout(0, 0, 1.0, 0, true, &info);
        assert_eq!((l.render_height, l.anim_width, l.stacked), (36, 60, false));
    }

    #[test]
    fn layout_stacks_on_narrow_terminals() {
        let info = vec!["x".repeat(30); 10];
        let l = apply_layout(50, 40, 1.0, 0, true, &info);
        assert!(l.stacked);
        assert_eq!(l.anim_width, 40);
        assert_eq!(l.info_clip_cols, 40);
    }
}
