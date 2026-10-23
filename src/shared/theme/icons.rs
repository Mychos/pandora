use egui::{Color32, RichText};
use std::collections::HashMap;

use super::brands::Brand;
use crate::shared::ui::responsive;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Icon {
    Plus,
    LockKeyhole,
    LockKeyholeOpen,
    Settings,
    Sliders,
    Search,
    Eye,
    EyeOff,
    Copy,
    User,
    Key,
    CreditCard,
    Fingerprint,
    Note,
    Paperclip,
    Trash,
    Back,
    Up,
    Download,
    X,
    Folder,
    FolderPlus,
    Globe,
    ChevronRight,
    Pencil,
    Dices,
    History,
    RefreshCw,
    LogOut,
    Upload,
    Puzzle,
    List,
    QrCode,
    Camera,
    File,
    FileImage,
    FileText,
    FileCode,
    FileAudio,
    FileVideo,
    FileArchive,
    Play,
    Pause,
    Rewind,
    Forward,
    Volume,
    Mute,
}

const ICON_FILES: &[(Icon, &str)] = &[
    (Icon::Plus, "plus"),
    (Icon::LockKeyhole, "lock-keyhole"),
    (Icon::LockKeyholeOpen, "lock-keyhole-open"),
    (Icon::Settings, "settings"),
    (Icon::Sliders, "sliders-horizontal"),
    (Icon::Search, "search"),
    (Icon::Eye, "eye"),
    (Icon::EyeOff, "eye-off"),
    (Icon::Copy, "copy"),
    (Icon::User, "user-round"),
    (Icon::Key, "key-round"),
    (Icon::CreditCard, "credit-card"),
    (Icon::Fingerprint, "fingerprint"),
    (Icon::Note, "sticky-note"),
    (Icon::Paperclip, "paperclip"),
    (Icon::Trash, "trash-2"),
    (Icon::Back, "arrow-left"),
    (Icon::Up, "arrow-up"),
    (Icon::Download, "download"),
    (Icon::X, "x"),
    (Icon::Folder, "folder"),
    (Icon::FolderPlus, "folder-plus"),
    (Icon::Globe, "globe"),
    (Icon::ChevronRight, "chevron-right"),
    (Icon::Pencil, "pencil"),
    (Icon::Dices, "dices"),
    (Icon::History, "history"),
    (Icon::RefreshCw, "refresh-cw"),
    (Icon::LogOut, "log-out"),
    (Icon::Upload, "upload"),
    (Icon::Puzzle, "puzzle"),
    (Icon::List, "list"),
    (Icon::QrCode, "qr-code"),
    (Icon::Camera, "camera"),
    (Icon::File, "file"),
    (Icon::FileImage, "file-image"),
    (Icon::FileText, "file-text"),
    (Icon::FileCode, "file-code"),
    (Icon::FileAudio, "file-audio"),
    (Icon::FileVideo, "file-video"),
    (Icon::FileArchive, "file-archive"),
    (Icon::Play, "play"),
    (Icon::Pause, "pause"),
    (Icon::Rewind, "rewind"),
    (Icon::Forward, "fast-forward"),
    (Icon::Volume, "volume-2"),
    (Icon::Mute, "volume-x"),
];

/// 28 px under a pointer; a phone asks for [`responsive::target`] instead.
pub const ICON: f32 = 28.0;

pub struct Icons {
    map: HashMap<Icon, egui::TextureHandle>,
    brands: HashMap<Brand, egui::TextureHandle>,
    logo: egui::TextureHandle,
}

impl Icons {
    pub fn load(ctx: &egui::Context) -> Self {
        let mut map = HashMap::new();
        for (icon, name) in ICON_FILES {
            if let Ok(img) = load_sprite(name) {
                map.insert(
                    *icon,
                    ctx.load_texture(format!("icon-{name}"), img, egui::TextureOptions::LINEAR),
                );
            }
        }
        let mut brands = HashMap::new();
        for brand in Brand::ALL {
            if let Ok(img) = load_brand_sprite(brand.slug()) {
                brands.insert(
                    brand,
                    ctx.load_texture(
                        format!("brand-{}", brand.slug()),
                        img,
                        egui::TextureOptions::LINEAR,
                    ),
                );
            }
        }
        Self {
            map,
            brands,
            logo: upload(ctx, "logo", include_bytes!("../../../assets/logo.png")),
        }
    }

    pub fn id(&self, icon: Icon) -> Option<egui::TextureId> {
        self.map.get(&icon).map(|t| t.id())
    }

    pub fn image(&self, icon: Icon, size: f32, color: Color32) -> Option<egui::Image<'static>> {
        self.map.get(&icon).map(|t| {
            egui::Image::new(egui::load::SizedTexture::new(
                t.id(),
                egui::vec2(size, size),
            ))
            .tint(color)
        })
    }

    pub fn brand_image(
        &self,
        brand: Brand,
        size: f32,
        color: Color32,
    ) -> Option<egui::Image<'static>> {
        self.brands.get(&brand).map(|t| {
            egui::Image::new(egui::load::SizedTexture::new(
                t.id(),
                egui::vec2(size, size),
            ))
            .tint(color)
        })
    }

    pub fn glyph(
        &self,
        brand: Option<Brand>,
        icon: Icon,
        size: f32,
        color: Color32,
    ) -> Option<egui::Image<'static>> {
        brand
            .and_then(|b| self.brand_image(b, size, color))
            .or_else(|| self.image(icon, size, color))
    }

    pub fn logo(&self, height: f32, color: Color32) -> egui::Image<'static> {
        let shape = self.logo.size_vec2();
        egui::Image::new(egui::load::SizedTexture::new(
            self.logo.id(),
            egui::vec2(height * shape.x / shape.y, height),
        ))
        .tint(color)
    }

    /// The whole response, so chrome that marks the button (the window bar) reaches its rect.
    pub fn button(
        &self,
        ui: &mut egui::Ui,
        icon: Icon,
        size: f32,
        color: Color32,
        tooltip: &str,
    ) -> egui::Response {
        self.framed(ui, icon, size, color, tooltip, responsive::target(ui.ctx()))
    }

    pub fn framed(
        &self,
        ui: &mut egui::Ui,
        icon: Icon,
        size: f32,
        color: Color32,
        tooltip: &str,
        side: f32,
    ) -> egui::Response {
        // A finger gets the whole square; a tighter mark keeps its size and only grows.
        let size = responsive::grow(ui.ctx(), size);
        let side = responsive::grow_target(ui.ctx(), side);
        let Some(img) = self.image(icon, size, color) else {
            return ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover());
        };
        // Whole pixels, or the margin rounds up and every square comes out a pixel
        // wider than `side`, overflowing the row that gave it room.
        let pad = ((side - size) / 2.0).floor().max(0.0);
        // A screen reader announces the tooltip, so an icon-only button needs it as alt text.
        let img = if tooltip.is_empty() {
            img
        } else {
            img.alt_text(crate::shared::i18n::text(tooltip).into_owned())
        };
        let mut resp = ui
            .allocate_ui_with_layout(
                egui::vec2(side, side),
                // egui aligns content by the ambient layout, so a row pinned to the top
                // would pin the icon; the square is fixed and the icon stays centred.
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.spacing_mut().button_padding = egui::vec2(pad, pad);
                    ui.add(
                        egui::Button::new(img)
                            .min_size(egui::vec2(side, side))
                            .small(),
                    )
                },
            )
            .inner;
        if !tooltip.is_empty() {
            resp = resp.on_hover_text(crate::shared::i18n::text(tooltip).into_owned());
        }
        resp
    }

    pub fn labeled(
        &self,
        ui: &mut egui::Ui,
        icon: Icon,
        size: f32,
        color: Color32,
        text: &str,
    ) -> egui::Response {
        let mut atoms = egui::Atoms::new(
            RichText::new(crate::shared::i18n::text(text).into_owned()).color(color),
        );
        // Only the glyph follows the target; `min_size` holds the button's height open.
        let glyph = responsive::grow(ui.ctx(), size);
        if let Some(img) = self.image(icon, glyph, color) {
            atoms.push_left(img);
        }
        ui.add(
            egui::Button::new(atoms)
                .small()
                .min_size(egui::vec2(0.0, responsive::target(ui.ctx()))),
        )
    }

    pub fn trailing(
        &self,
        icon: Option<Icon>,
        size: f32,
        color: Color32,
        label: RichText,
    ) -> egui::Atoms<'static> {
        let mut atoms = egui::Atoms::new(label);
        if let Some(img) = icon.and_then(|i| self.image(i, size, color)) {
            atoms.push_right(img);
        }
        atoms
    }
}

/// OS window and taskbar icon: the white mark already composited on the accent tile
/// by `scripts/build-logo.sh`, because a window manager cannot tint it.
pub fn window_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
        .expect("bundled app icon")
        .to_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

fn upload(ctx: &egui::Context, name: &str, bytes: &'static [u8]) -> egui::TextureHandle {
    ctx.load_texture(
        name,
        decode(bytes).expect("bundled logo png"),
        egui::TextureOptions::LINEAR,
    )
}

fn load_sprite(name: &str) -> Result<egui::ColorImage, image::ImageError> {
    // include_bytes! needs a literal path, so match against them.
    let bytes: &'static [u8] = match name {
        "plus" => include_bytes!("../../../assets/icons/plus.png"),
        "lock-keyhole" => include_bytes!("../../../assets/icons/lock-keyhole.png"),
        "lock-keyhole-open" => include_bytes!("../../../assets/icons/lock-keyhole-open.png"),
        "settings" => include_bytes!("../../../assets/icons/settings.png"),
        "sliders-horizontal" => include_bytes!("../../../assets/icons/sliders-horizontal.png"),
        "search" => include_bytes!("../../../assets/icons/search.png"),
        "eye" => include_bytes!("../../../assets/icons/eye.png"),
        "eye-off" => include_bytes!("../../../assets/icons/eye-off.png"),
        "copy" => include_bytes!("../../../assets/icons/copy.png"),
        "user-round" => include_bytes!("../../../assets/icons/user-round.png"),
        "key-round" => include_bytes!("../../../assets/icons/key-round.png"),
        "credit-card" => include_bytes!("../../../assets/icons/credit-card.png"),
        "fingerprint" => include_bytes!("../../../assets/icons/fingerprint.png"),
        "sticky-note" => include_bytes!("../../../assets/icons/sticky-note.png"),
        "paperclip" => include_bytes!("../../../assets/icons/paperclip.png"),
        "trash-2" => include_bytes!("../../../assets/icons/trash-2.png"),
        "arrow-left" => include_bytes!("../../../assets/icons/arrow-left.png"),
        "arrow-up" => include_bytes!("../../../assets/icons/arrow-up.png"),
        "download" => include_bytes!("../../../assets/icons/download.png"),
        "x" => include_bytes!("../../../assets/icons/x.png"),
        "folder" => include_bytes!("../../../assets/icons/folder.png"),
        "folder-plus" => include_bytes!("../../../assets/icons/folder-plus.png"),
        "globe" => include_bytes!("../../../assets/icons/globe.png"),
        "chevron-right" => include_bytes!("../../../assets/icons/chevron-right.png"),
        "pencil" => include_bytes!("../../../assets/icons/pencil.png"),
        "dices" => include_bytes!("../../../assets/icons/dices.png"),
        "history" => include_bytes!("../../../assets/icons/history.png"),
        "refresh-cw" => include_bytes!("../../../assets/icons/refresh-cw.png"),
        "log-out" => include_bytes!("../../../assets/icons/log-out.png"),
        "upload" => include_bytes!("../../../assets/icons/upload.png"),
        "puzzle" => include_bytes!("../../../assets/icons/puzzle.png"),
        "list" => include_bytes!("../../../assets/icons/list.png"),
        "qr-code" => include_bytes!("../../../assets/icons/qr-code.png"),
        "camera" => include_bytes!("../../../assets/icons/camera.png"),
        "file" => include_bytes!("../../../assets/icons/file.png"),
        "file-image" => include_bytes!("../../../assets/icons/file-image.png"),
        "file-text" => include_bytes!("../../../assets/icons/file-text.png"),
        "file-code" => include_bytes!("../../../assets/icons/file-code.png"),
        "file-audio" => include_bytes!("../../../assets/icons/file-audio.png"),
        "file-video" => include_bytes!("../../../assets/icons/file-video.png"),
        "file-archive" => include_bytes!("../../../assets/icons/file-archive.png"),
        "play" => include_bytes!("../../../assets/icons/play.png"),
        "pause" => include_bytes!("../../../assets/icons/pause.png"),
        "rewind" => include_bytes!("../../../assets/icons/rewind.png"),
        "fast-forward" => include_bytes!("../../../assets/icons/fast-forward.png"),
        "volume-2" => include_bytes!("../../../assets/icons/volume-2.png"),
        "volume-x" => include_bytes!("../../../assets/icons/volume-x.png"),
        _ => return Ok(egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT])),
    };
    decode(bytes)
}

fn load_brand_sprite(slug: &str) -> Result<egui::ColorImage, image::ImageError> {
    let bytes: &'static [u8] = match slug {
        "github" => include_bytes!("../../../assets/icons/brands/github.png"),
        "gitlab" => include_bytes!("../../../assets/icons/brands/gitlab.png"),
        "bitbucket" => include_bytes!("../../../assets/icons/brands/bitbucket.png"),
        "google" => include_bytes!("../../../assets/icons/brands/google.png"),
        "apple" => include_bytes!("../../../assets/icons/brands/apple.png"),
        "microsoft" => include_bytes!("../../../assets/icons/brands/microsoft.png"),
        "amazon" => include_bytes!("../../../assets/icons/brands/amazon.png"),
        "amazonaws" => include_bytes!("../../../assets/icons/brands/amazonaws.png"),
        "netflix" => include_bytes!("../../../assets/icons/brands/netflix.png"),
        "spotify" => include_bytes!("../../../assets/icons/brands/spotify.png"),
        "discord" => include_bytes!("../../../assets/icons/brands/discord.png"),
        "slack" => include_bytes!("../../../assets/icons/brands/slack.png"),
        "twitch" => include_bytes!("../../../assets/icons/brands/twitch.png"),
        "youtube" => include_bytes!("../../../assets/icons/brands/youtube.png"),
        "x" => include_bytes!("../../../assets/icons/brands/x.png"),
        "instagram" => include_bytes!("../../../assets/icons/brands/instagram.png"),
        "facebook" => include_bytes!("../../../assets/icons/brands/facebook.png"),
        "linkedin" => include_bytes!("../../../assets/icons/brands/linkedin.png"),
        "telegram" => include_bytes!("../../../assets/icons/brands/telegram.png"),
        "whatsapp" => include_bytes!("../../../assets/icons/brands/whatsapp.png"),
        "dropbox" => include_bytes!("../../../assets/icons/brands/dropbox.png"),
        "googledrive" => include_bytes!("../../../assets/icons/brands/googledrive.png"),
        "microsoftonedrive" => include_bytes!("../../../assets/icons/brands/microsoftonedrive.png"),
        "protonmail" => include_bytes!("../../../assets/icons/brands/protonmail.png"),
        "paypal" => include_bytes!("../../../assets/icons/brands/paypal.png"),
        "steam" => include_bytes!("../../../assets/icons/brands/steam.png"),
        "reddit" => include_bytes!("../../../assets/icons/brands/reddit.png"),
        "cloudflare" => include_bytes!("../../../assets/icons/brands/cloudflare.png"),
        "ebay" => include_bytes!("../../../assets/icons/brands/ebay.png"),
        "shopify" => include_bytes!("../../../assets/icons/brands/shopify.png"),
        "zoom" => include_bytes!("../../../assets/icons/brands/zoom.png"),
        "stripe" => include_bytes!("../../../assets/icons/brands/stripe.png"),
        _ => return Ok(egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT])),
    };
    decode(bytes)
}

/// A frame of live video reuses one texture slot, so a picture redrawn every frame
/// costs no allocation.
pub fn live_texture(
    ctx: &egui::Context,
    id: egui::Id,
    image: egui::ColorImage,
) -> egui::TextureHandle {
    if let Some(mut tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        tex.set(image, egui::TextureOptions::LINEAR);
        return tex;
    }
    let tex = ctx.load_texture(format!("{id:?}"), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// The frame last handed to [`live_texture`], for the frames in between.
pub fn live_frame(ctx: &egui::Context, id: egui::Id) -> Option<egui::TextureHandle> {
    ctx.data(|d| d.get_temp::<egui::TextureHandle>(id))
}

/// Bytes of any supported image (png, jpeg, webp) → pixels.
pub fn decode(bytes: &[u8]) -> Result<egui::ColorImage, image::ImageError> {
    let img = image::load_from_memory(bytes)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        rgba.as_raw(),
    ))
}

pub fn kind_icon(kind: crate::vault::model::Kind) -> Icon {
    use crate::vault::model::Kind;
    match kind {
        Kind::Login => Icon::Key,
        Kind::Card => Icon::CreditCard,
        Kind::Identity => Icon::User,
        Kind::Passkey => Icon::Fingerprint,
        Kind::Note => Icon::Note,
        Kind::File => Icon::Paperclip,
        Kind::Otp => Icon::RefreshCw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_name_loads_its_sprite() {
        for (_, name) in ICON_FILES {
            let img = load_sprite(name).expect(name);
            assert_eq!(img.size, [64, 64], "{name} is not a 64x64 sprite");
        }
    }
}
