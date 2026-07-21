use super::catalog::cache_directory;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

const MAX_INFLIGHT: usize = 3;
const ICON_PIXELS: u32 = 64;

enum IconMessage {
    Ready { url: String, image: ColorImage },
    Failed { url: String },
}

pub struct IconCache {
    textures: HashMap<String, TextureHandle>,
    inflight: HashSet<String>,
    failed: HashSet<String>,
    queued: VecDeque<String>,
    queued_set: HashSet<String>,
    tx: Sender<IconMessage>,
    rx: Receiver<IconMessage>,
}

impl Default for IconCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            textures: HashMap::new(),
            inflight: HashSet::new(),
            failed: HashSet::new(),
            queued: VecDeque::new(),
            queued_set: HashSet::new(),
            tx,
            rx,
        }
    }
}

impl IconCache {
    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut received = false;
        loop {
            match self.rx.try_recv() {
                Ok(IconMessage::Ready { url, image }) => {
                    self.inflight.remove(&url);
                    let texture = ctx.load_texture(
                        format!("nexus-mod-icon-{}", url_hash(&url)),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.textures.insert(url, texture);
                    received = true;
                }
                Ok(IconMessage::Failed { url }) => {
                    self.inflight.remove(&url);
                    self.failed.insert(url);
                    received = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        self.pump(ctx);
        if received {
            ctx.request_repaint();
        }
    }

    pub fn texture_for(&mut self, ctx: &egui::Context, url: &str) -> Option<TextureHandle> {
        if url.is_empty() || self.failed.contains(url) {
            return None;
        }
        if let Some(texture) = self.textures.get(url) {
            return Some(texture.clone());
        }
        if self.inflight.contains(url) {
            return None;
        }
        if self.queued_set.contains(url) {
            if let Some(index) = self.queued.iter().position(|entry| entry == url) {
                if let Some(entry) = self.queued.remove(index) {
                    self.queued.push_front(entry);
                }
            }
            self.pump(ctx);
            return None;
        }

        self.queued.push_front(url.to_owned());
        self.queued_set.insert(url.to_owned());
        self.pump(ctx);
        None
    }

    fn pump(&mut self, ctx: &egui::Context) {
        while self.inflight.len() < MAX_INFLIGHT {
            let Some(url) = self.queued.pop_front() else {
                break;
            };
            self.queued_set.remove(&url);
            if self.textures.contains_key(&url)
                || self.failed.contains(&url)
                || self.inflight.contains(&url)
            {
                continue;
            }
            self.spawn_load(ctx, &url);
        }
    }

    fn spawn_load(&mut self, ctx: &egui::Context, url: &str) {
        self.inflight.insert(url.to_owned());
        let tx = self.tx.clone();
        let url_owned = url.to_owned();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let message = match load_icon_image(&url_owned) {
                Ok(image) => IconMessage::Ready {
                    url: url_owned,
                    image,
                },
                Err(_) => IconMessage::Failed { url: url_owned },
            };
            let _ = tx.send(message);
            ctx.request_repaint();
        });
    }
}

fn load_icon_image(url: &str) -> Result<ColorImage, String> {
    if let Some(image) = read_cached_webp(url)? {
        return Ok(image);
    }
    if let Some(image) = read_legacy_png_cache(url)? {
        let _ = store_cached_webp(url, &image);
        return Ok(image);
    }

    let bytes = download_icon(url)?;
    let image = decode_and_resize(&bytes)?;
    let _ = store_cached_webp(url, &image);
    Ok(image)
}

fn url_hash(url: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn icon_cache_dir() -> Option<PathBuf> {
    Some(cache_directory().ok()?.join("icons"))
}

fn webp_cache_path(url: &str) -> Option<PathBuf> {
    Some(icon_cache_dir()?.join(format!("{}.webp", url_hash(url))))
}

fn legacy_png_cache_path(url: &str) -> Option<PathBuf> {
    Some(icon_cache_dir()?.join(format!("{}.png", url_hash(url))))
}

fn read_cached_webp(url: &str) -> Result<Option<ColorImage>, String> {
    let Some(path) = webp_cache_path(url) else {
        return Ok(None);
    };
    let Ok(bytes) = fs::read(path) else {
        return Ok(None);
    };
    Ok(Some(decode_and_resize(&bytes)?))
}

fn read_legacy_png_cache(url: &str) -> Result<Option<ColorImage>, String> {
    let Some(path) = legacy_png_cache_path(url) else {
        return Ok(None);
    };
    let Ok(bytes) = fs::read(&path) else {
        return Ok(None);
    };
    let image = decode_and_resize(&bytes)?;
    let _ = fs::remove_file(path);
    Ok(Some(image))
}

fn store_cached_webp(url: &str, image: &ColorImage) -> Result<(), String> {
    let path = webp_cache_path(url).ok_or_else(|| "icon cache path unavailable".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let rgba = image
        .pixels
        .iter()
        .flat_map(|pixel| {
            let [r, g, b, a] = pixel.to_array();
            [r, g, b, a]
        })
        .collect::<Vec<u8>>();
    let dynamic =
        image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, rgba)
            .ok_or_else(|| "failed to rebuild rgba image for cache".to_string())?;

    let mut encoded = Vec::new();
    DynamicImage::ImageRgba8(dynamic)
        .write_to(&mut Cursor::new(&mut encoded), ImageFormat::WebP)
        .map_err(|error| format!("webp encode: {error}"))?;

    let temporary = path.with_extension("webp.partial");
    fs::write(&temporary, &encoded).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())?;
    Ok(())
}

fn download_icon(url: &str) -> Result<Vec<u8>, String> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-time",
            "15",
            "--compressed",
        ])
        .arg(url)
        .output()
        .map_err(|error| format!("failed to spawn curl: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(if stderr.trim().is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr.trim().to_string()
        });
    }

    if output.stdout.is_empty() {
        return Err("empty icon response".to_string());
    }

    Ok(output.stdout)
}

fn decode_and_resize(bytes: &[u8]) -> Result<ColorImage, String> {
    let dynamic = image::load_from_memory(bytes).map_err(|error| format!("decode: {error}"))?;
    let resized = dynamic.resize_exact(ICON_PIXELS, ICON_PIXELS, FilterType::Triangle);
    let rgba = resized.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [ICON_PIXELS as usize, ICON_PIXELS as usize],
        rgba.as_raw(),
    ))
}
