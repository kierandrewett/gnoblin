use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::xdg;

fn load_icon_path_sized(path: &Path, target_size: Option<u32>) -> Option<slint::Image> {
    if !path.exists() {
        return None;
    }
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    if path.extension().is_none() {
        if let Some(img) = load_raster_icon_path(path, target_size) {
            return Some(img);
        }
    } else if ext.as_deref() == Some("xpm") {
        return load_xpm_icon_path(path);
    } else if target_size.is_some() && matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg")) {
        if let Some(img) = load_raster_icon_path(path, target_size) {
            return Some(img);
        }
    }
    if let Ok(img) = slint::Image::load_from_path(path) {
        return Some(img);
    }
    load_raster_icon_path(path, target_size).or_else(|| load_xpm_icon_path(path))
}

fn load_raster_icon_path(path: &Path, target_size: Option<u32>) -> Option<slint::Image> {
    let bytes = std::fs::read(path).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?;
    if let Some(size) = target_size.filter(|size| *size > 0) {
        if img.width() != size || img.height() != size {
            img = img.resize(size, size, image::imageops::FilterType::Lanczos3);
        }
    }
    let img = img.to_rgba8();
    let (w, h) = img.dimensions();
    let buf =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&img.into_raw(), w, h);
    Some(slint::Image::from_rgba8(buf))
}

fn load_xpm_icon_path(path: &Path) -> Option<slint::Image> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines = xpm_string_lines(&text);
    let header = lines.first()?;
    let mut header_parts = header.split_whitespace();
    let width: usize = header_parts.next()?.parse().ok()?;
    let height: usize = header_parts.next()?.parse().ok()?;
    let color_count: usize = header_parts.next()?.parse().ok()?;
    let chars_per_pixel: usize = header_parts.next()?.parse().ok()?;
    if width == 0 || height == 0 || chars_per_pixel == 0 || width > 4096 || height > 4096 {
        return None;
    }
    if lines.len() < 1 + color_count + height {
        return None;
    }

    let mut colors = HashMap::new();
    for line in &lines[1..1 + color_count] {
        let (key, rest) = split_xpm_key(line, chars_per_pixel)?;
        let color = parse_xpm_color(rest)?;
        colors.insert(key, color);
    }

    let mut rgba = Vec::with_capacity(width.checked_mul(height)?.checked_mul(4)?);
    for line in &lines[1 + color_count..1 + color_count + height] {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < width.checked_mul(chars_per_pixel)? {
            return None;
        }
        for x in 0..width {
            let start = x * chars_per_pixel;
            let key: String = chars[start..start + chars_per_pixel].iter().collect();
            rgba.extend_from_slice(colors.get(&key)?);
        }
    }

    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &rgba,
        width as u32,
        height as u32,
    );
    Some(slint::Image::from_rgba8(buf))
}

fn xpm_string_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in text.chars() {
        if in_string {
            if escape {
                current.push(ch);
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                lines.push(std::mem::take(&mut current));
                in_string = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            in_string = true;
        }
    }
    lines
}

fn split_xpm_key(line: &str, chars_per_pixel: usize) -> Option<(String, &str)> {
    let mut end = 0;
    for (count, (idx, ch)) in line.char_indices().enumerate() {
        if count == chars_per_pixel {
            break;
        }
        end = idx + ch.len_utf8();
    }
    if line[..end].chars().count() != chars_per_pixel {
        return None;
    }
    Some((line[..end].to_string(), &line[end..]))
}

fn parse_xpm_color(rest: &str) -> Option<[u8; 4]> {
    let mut tokens = rest.split_whitespace();
    while let Some(token) = tokens.next() {
        if token != "c" {
            continue;
        }
        let color = tokens.next()?;
        if color.eq_ignore_ascii_case("none") {
            return Some([0, 0, 0, 0]);
        }
        return parse_xpm_hex_color(color);
    }
    None
}

fn parse_xpm_hex_color(color: &str) -> Option<[u8; 4]> {
    let hex = color.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let r = parse_xpm_nibble(chars.next()?)? * 17;
            let g = parse_xpm_nibble(chars.next()?)? * 17;
            let b = parse_xpm_nibble(chars.next()?)? * 17;
            Some([r, g, b, 255])
        }
        6 => Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255,
        ]),
        _ => None,
    }
}

fn parse_xpm_nibble(ch: char) -> Option<u8> {
    ch.to_digit(16).map(|v| v as u8)
}

/// Resolve an icon by name (or absolute path) from an explicit theme path then
/// the system icon theme dirs -> a Slint image. Used by the dock + the SNI tray.
pub fn find_icon(name: &str, theme_path: &str) -> Option<slint::Image> {
    find_icon_internal(name, theme_path, None)
}

/// Resolve an icon like [`find_icon`], but pre-render raster sources near the
/// requested logical size. This mirrors GTK/GNOME's size-aware icon path and
/// avoids leaving large app icons for the renderer to downsample every frame.
pub fn find_icon_at_size(name: &str, theme_path: &str, logical_size: u32) -> Option<slint::Image> {
    find_icon_internal(name, theme_path, Some(logical_size.max(1)))
}

// (name, theme_path, target_size) → resolved image (or None, cached so misses
// aren't re-searched).
type IconCache = std::collections::HashMap<(String, String, Option<u32>), Option<slint::Image>>;

thread_local! {
    // Resolved-icon cache: the theme search stats dozens of paths per lookup, and
    // find_icon runs for every tile + submenu row on every control-centre refresh,
    // so an uncached lookup is a real open-time cost. slint::Image is cheap to
    // clone (ref-counted).
    static ICON_CACHE: std::cell::RefCell<IconCache> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn find_icon_internal(
    name: &str,
    theme_path: &str,
    target_size: Option<u32>,
) -> Option<slint::Image> {
    let key = (name.to_string(), theme_path.to_string(), target_size);
    if let Some(hit) = ICON_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let result = find_icon_uncached(name, theme_path, target_size);
    inspect_log_icon(name, theme_path, target_size, &result);
    ICON_CACHE.with(|c| c.borrow_mut().insert(key, result.clone()));
    result
}

/// When `GNOBLIN_INSPECT` is set, append each unique icon resolution (logged
/// once, on cache miss) to `$XDG_RUNTIME_DIR/gnoblin-inspect/icons-<pid>.jsonl`
/// so the inspector can see exactly which icon resolved for each name/size and at
/// what pixel dimensions — the Slint-side counterpart to the compositor's scene
/// dump. No-op unless the env var is set.
fn inspect_log_icon(
    name: &str,
    theme_path: &str,
    size: Option<u32>,
    result: &Option<slint::Image>,
) {
    if std::env::var_os("GNOBLIN_INSPECT").is_none() {
        return;
    }
    let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => std::path::PathBuf::from(d).join("gnoblin-inspect"),
        None => return,
    };
    let _ = std::fs::create_dir_all(&dir);
    let dims = result
        .as_ref()
        .map(|img| {
            let s = img.size();
            format!("[{},{}]", s.width, s.height)
        })
        .unwrap_or_else(|| "null".to_string());
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let line = format!(
        "{{\"name\":\"{}\",\"theme\":\"{}\",\"req_size\":{},\"resolved\":{},\"dims\":{}}}\n",
        esc(name),
        esc(theme_path),
        size.map(|s| s.to_string()).unwrap_or_else(|| "null".into()),
        result.is_some(),
        dims,
    );
    use std::io::Write;
    let path = dir.join(format!("icons-{}.jsonl", std::process::id()));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn find_icon_uncached(
    name: &str,
    theme_path: &str,
    target_size: Option<u32>,
) -> Option<slint::Image> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return load_icon_path_sized(Path::new(name), target_size);
    }
    if !theme_path.is_empty() {
        for ext in ["svg", "png", "xpm"] {
            let p = format!("{theme_path}/{name}.{ext}");
            if let Some(img) = load_icon_path_sized(Path::new(&p), target_size) {
                return Some(img);
            }
        }
    }
    let mut theme_dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        for theme in ["hicolor", "Adwaita", "breeze"] {
            theme_dirs.push(PathBuf::from(&home).join(".icons").join(theme));
        }
    }
    if let Some(data_home) = xdg::data_home() {
        for theme in ["hicolor", "Adwaita", "breeze"] {
            theme_dirs.push(data_home.join("icons").join(theme));
        }
    }
    for data_dir in xdg::data_dirs() {
        for theme in ["hicolor", "Adwaita", "breeze"] {
            theme_dirs.push(data_dir.join("icons").join(theme));
        }
    }
    const SIZES: &[&str] = &[
        "scalable", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "24x24", "symbolic",
    ];
    for theme in theme_dirs {
        for size in SIZES {
            for cat in ["apps", "status", "devices"] {
                for ext in ["svg", "png", "xpm"] {
                    let p = theme.join(size).join(cat).join(format!("{name}.{ext}"));
                    if p.exists() {
                        if let Some(img) = load_icon_path_sized(&p, target_size) {
                            return Some(img);
                        }
                    }
                }
            }
        }
    }
    for ext in ["svg", "png", "xpm"] {
        let p = format!("/usr/share/pixmaps/{name}.{ext}");
        if let Some(img) = load_icon_path_sized(Path::new(&p), target_size) {
            return Some(img);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, temp_root, EnvVar};

    #[test]
    fn find_icon_searches_xdg_data_home_hicolor_theme() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-xdg-data-home");
        let data_home = root.join("data");
        let icon_dir = data_home.join("icons/hicolor/48x48/apps");
        std::fs::create_dir_all(&icon_dir).unwrap();
        let mut img = image::RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([0, 128, 255, 255]));
        img.save(icon_dir.join("local-only.png")).unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert!(find_icon("local-only", "").is_some());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_at_size_downsamples_large_raster_icons() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-sized-raster");
        let data_home = root.join("data");
        let icon_dir = data_home.join("icons/hicolor/256x256/apps");
        std::fs::create_dir_all(&icon_dir).unwrap();
        let mut img = image::RgbaImage::new(256, 256);
        for y in 0..256 {
            for x in 0..256 {
                img.put_pixel(x, y, image::Rgba([x as u8, y as u8, 192, 255]));
            }
        }
        img.save(icon_dir.join("large-only.png")).unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        let icon = find_icon_at_size("large-only", "", 48).unwrap();
        let size = icon.size();
        assert_eq!((size.width, size.height), (48, 48));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_loads_absolute_extensionless_png() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-extensionless-absolute");
        std::fs::create_dir_all(&root).unwrap();
        let icon = root.join("app-icon");
        let mut img = image::RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([255, 64, 0, 255]));
        img.write_to(
            &mut std::io::BufWriter::new(std::fs::File::create(&icon).unwrap()),
            image::ImageOutputFormat::Png,
        )
        .unwrap();

        assert!(find_icon(icon.to_str().unwrap(), "").is_some());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_loads_xpm_icons() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-xpm");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("local-xpm.xpm"),
            r##"/* XPM */
static const char * local_xpm[] = {
"2 2 3 1",
"  c None",
". c #ff0000",
"+ c #00ff00",
".+",
"+."
};
"##,
        )
        .unwrap();

        assert!(find_icon("local-xpm", root.to_str().unwrap()).is_some());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
