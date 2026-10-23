//! Creating the codes of the box on screen, and reading one out of a picture.

use crate::app::StatusKind;
use crate::features::boxes::model::Boxes;
#[cfg(target_os = "android")]
use crate::features::codes::model::Reading;
use crate::features::codes::model::Scan;
use crate::features::shell::model::Shell;
use crate::vault::model::{now_secs, Otp};
use crate::vault::totp;
use zeroize::Zeroizing;

impl Boxes {
    /// A code, created in the box on screen and selected. The secret starts
    /// empty, so the row says what it still needs instead of showing nothing.
    pub fn add_otp(&mut self) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let it = Otp::new(now_secs());
        let id = it.id.clone();
        s.vault.otps.push(it);
        s.selected = Some(id);
        s.mark_now();
    }

    /// Open the camera for a scan that reads as the user aims. Answers the frame
    /// it will hand over, so the pane knows what it is looking at; a camera that
    /// will not open says why in the status line, in its own words.
    #[cfg(target_os = "android")]
    pub fn start_scan(&mut self, shell: &mut Shell) -> bool {
        let shape = match crate::shared::android::text("startCamera") {
            Ok(Some(shape)) => shape,
            Ok(None) => {
                shell.fail(crate::shared::i18n::t("the camera did not answer"));
                return false;
            }
            Err(e) => {
                shell.fail(e);
                return false;
            }
        };
        let Some((width, height, turn)) = shape_of(&shape) else {
            // Anything that is not a shape is the camera's own sentence about
            // why there is none, and it is the one worth showing.
            let _ = crate::shared::android::send_none("stopCamera");
            shell.fail(shape);
            return false;
        };
        let Some(s) = self.session.as_mut() else {
            let _ = crate::shared::android::send_none("stopCamera");
            return false;
        };
        s.scan = Some(Scan::new(width, height, turn));
        true
    }

    /// Close the camera. The pane stops polling the moment the scan is gone.
    #[cfg(target_os = "android")]
    pub fn stop_scan(&mut self) {
        if let Some(s) = self.session.as_mut() {
            s.scan = None;
        }
        let _ = crate::shared::android::send_none("stopCamera");
    }

    /// Take the frame the camera has ready, if this moment is the one for it: the
    /// preview is refreshed, and every third frame goes to the reader — on its own
    /// thread, one at a time, so a slow frame never costs the picture on screen.
    /// Answers the seed the reader came back with, if any.
    #[cfg(target_os = "android")]
    pub fn poll_scan(&mut self, ctx: &egui::Context, now: std::time::Instant) -> Option<Otp> {
        let mut found = None;
        {
            let s = self.session.as_mut()?;
            let scan = s.scan.as_mut()?;
            // Whatever the reader found since the last frame.
            if let Some(done) = scan.reading.as_mut() {
                match done.0.try_recv() {
                    Ok(answer) => {
                        found = answer;
                        scan.reading = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => scan.reading = None,
                }
            }
            if let Some(read) = scan.take(now) {
                if let Some(nv21) = crate::shared::android::bytes("cameraFrame").ok().flatten() {
                    if let Some((rgba, width, height)) = view(&nv21, scan) {
                        let _ = crate::shared::theme::live_texture(
                            ctx,
                            egui::Id::new("scan-frame"),
                            egui::ColorImage::from_rgba_unmultiplied(
                                [width as usize, height as usize],
                                &rgba,
                            ),
                        );
                    }
                    if read && scan.reading.is_none() {
                        if let Some(img) = luma(&nv21, scan) {
                            let (send, receive) = std::sync::mpsc::channel();
                            scan.reading = Some(Reading(receive));
                            std::thread::spawn(move || {
                                let _ = send.send(decode(img).ok());
                            });
                        }
                    }
                }
            }
        }
        let (name, seed) = found?;
        let mut it = Otp::new(now_secs());
        if let Some(name) = name {
            it.name = name;
        }
        it.secret = Zeroizing::new(seed);
        Some(it)
    }

    /// Read a one-time seed out of a QR: a picture the user picks, whatever a
    /// screenshot left on the clipboard, or a photo of another screen. Either way
    /// it lands in the box on screen, named after the URI when the URI says its
    /// name.
    ///
    /// A phone has no dialog to open and no clipboard image to read, so its picker
    /// is the photo one: a screenshot of its own screen. A QR on another screen is
    /// what `start_scan` is for, which reads as the camera is aimed.
    #[cfg(target_os = "android")]
    pub fn scan_code(&mut self, shell: &mut Shell) {
        self.land_picked(shell, crate::shared::android::text_with("pick", "image/*"));
    }

    /// What a phone's picker answered: a path to a copy it made, which has done
    /// its job once read — the picture itself is still on the phone.
    #[cfg(target_os = "android")]
    fn land_picked(&mut self, shell: &mut Shell, path: Result<Option<String>, String>) {
        let path = match path {
            Ok(Some(path)) => std::path::PathBuf::from(path),
            Ok(None) => return,
            Err(e) => return shell.fail(e),
        };
        self.land_file(shell, &path);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(not(target_os = "android"))]
    pub fn scan_code(&mut self, shell: &mut Shell) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        else {
            return;
        };
        self.land_file(shell, &path);
    }

    /// The same, from the clipboard: every desktop's own screenshot tool covers
    /// the screen, the user drags over the QR, and what is left there is read
    /// here. One path for every system with a clipboard — a phone reaches the
    /// photo picker and the camera instead.
    #[cfg(not(target_os = "android"))]
    pub fn grab_code(&mut self, shell: &mut Shell) {
        let picture = clipboard_image().and_then(|held| {
            held.ok_or_else(|| crate::shared::i18n::t("the clipboard holds no image").to_owned())
        });
        self.land_picture(shell, picture);
    }

    /// A picture the platform handed over as a path — a phone's picker and camera
    /// both answer that way, and so does the desktop's file dialog. Nothing is
    /// removed here: the path may be the user's own file.
    fn land_file(&mut self, shell: &mut Shell, path: &std::path::Path) {
        let picture = std::fs::read(path)
            .map_err(|e| {
                crate::shared::i18n::tf(
                    "could not read {name}: {error}",
                    &[("name", &path.display()), ("error", &e)],
                )
            })
            .and_then(|bytes| {
                image::load_from_memory(&bytes).map_err(|e| {
                    crate::shared::i18n::tf("that image cannot be read: {error}", &[("error", &e)])
                })
            });
        self.land_picture(shell, picture);
    }

    fn land_picture(&mut self, shell: &mut Shell, picture: Result<image::DynamicImage, String>) {
        match picture.and_then(decode) {
            Ok((name, seed)) => {
                self.land_code(name, seed);
                shell.set_status("code read from the image", StatusKind::Saved);
            }
            Err(e) => shell.fail(e),
        }
    }

    /// Put a code in the box on screen — one read from a picture, or one the
    /// camera just handed over — and select it.
    pub fn land(&mut self, otp: Otp) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        let id = otp.id.clone();
        s.vault.otps.push(otp);
        s.selected = Some(id);
        s.mark_now();
    }

    /// The same, for a seed that came with the name its URI carries.
    fn land_code(&mut self, name: Option<String>, seed: String) {
        let mut it = Otp::new(now_secs());
        if let Some(name) = name {
            it.name = name;
        }
        it.secret = Zeroizing::new(seed);
        self.land(it);
    }
}

/// Whatever image the clipboard holds; `None` when it holds none, which is every
/// clip that is not a picture.
#[cfg(not(target_os = "android"))]
fn clipboard_image() -> Result<Option<image::DynamicImage>, String> {
    let mut board = arboard::Clipboard::new().map_err(|e| {
        crate::shared::i18n::tf("cannot read the clipboard: {error}", &[("error", &e)])
    })?;
    match board.get_image() {
        Ok(held) => Ok(Some(as_image(
            held.width as u32,
            held.height as u32,
            held.bytes.into_owned(),
        )?)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(e) => Err(crate::shared::i18n::tf(
            "cannot read the clipboard: {error}",
            &[("error", &e)],
        )),
    }
}

/// Pixels as the clipboard hands them over — straight RGBA, no header, no
/// encoding — turned into an image, so the reader sees what a file would give it.
#[cfg(not(target_os = "android"))]
fn as_image(width: u32, height: u32, rgba: Vec<u8>) -> Result<image::DynamicImage, String> {
    image::RgbaImage::from_raw(width, height, rgba)
        .map(image::DynamicImage::ImageRgba8)
        .ok_or_else(|| crate::shared::i18n::t("the clipboard image is malformed").to_owned())
}

/// What the camera answered: `WxH <degrees>`, its own words for what it will
/// hand over.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn shape_of(shape: &str) -> Option<(u32, u32, u32)> {
    let (size, turn) = shape.trim().split_once([' ', '\t'])?;
    let (width, height) = size.split_once(['x', 'X'])?;
    Some((
        width.trim().parse().ok()?,
        height.trim().parse().ok()?,
        turn.trim().parse().ok()?,
    ))
}

/// The luma of a frame, which is what the reader wants and simply the first
/// `w*h` bytes of it: NV21 keeps every luma there, so this is no conversion at
/// all. `None` for a frame that is not the whole one.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn luma(nv21: &[u8], scan: &Scan) -> Option<image::DynamicImage> {
    let (width, height) = (scan.width, scan.height);
    let gray = image::GrayImage::from_raw(
        width,
        height,
        nv21.get(..(width * height) as usize)?.to_vec(),
    )?;
    Some(image::DynamicImage::ImageLuma8(gray))
}

/// The frame in colour and standing up, ready for a texture — at **half** the
/// camera's size, on purpose. A preview is looked at, not read, and a 1080p one
/// costs a colour conversion and a texture upload of eight megabytes ten times a
/// second, which is exactly how a preview stops being fluid. The reader gets the
/// frame's own pixels, untouched, and does its own smoothing.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn view(nv21: &[u8], scan: &Scan) -> Option<(Vec<u8>, u32, u32)> {
    let (width, height) = (scan.width as usize, scan.height as usize);
    if width < 2 || height < 2 || nv21.len() < width * height * 3 / 2 {
        return None;
    }
    let lit = &nv21[..width * height];
    let chroma = &nv21[width * height..];
    let (half_w, half_h) = (width / 2, height / 2);
    let mut rgb = image::RgbImage::new(half_w as u32, half_h as u32);
    for y in 0..half_h {
        for x in 0..half_w {
            // Four luma and one chroma pair per pixel: the camera's own
            // subsampling, averaged instead of interpolated.
            let mut sum = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let at = (y * 2 + dy) * width + (x * 2 + dx);
                    sum += u32::from(lit[at]);
                }
            }
            let luma = (sum / 4) as f32;
            let pair = y * width + x * 2;
            let v = chroma.get(pair).copied().unwrap_or(128) as f32 - 128.0;
            let u = chroma.get(pair + 1).copied().unwrap_or(128) as f32 - 128.0;
            // The usual integer-friendly form of the BT.601 matrix.
            let pixel = [
                (luma + 1.402 * v).clamp(0.0, 255.0) as u8,
                (luma - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8,
                (luma + 1.772 * u).clamp(0.0, 255.0) as u8,
            ];
            rgb.put_pixel(x as u32, y as u32, image::Rgb(pixel));
        }
    }
    let turned = match scan.turn % 360 {
        90 => image::imageops::rotate90(&rgb),
        180 => image::imageops::rotate180(&rgb),
        270 => image::imageops::rotate270(&rgb),
        _ => rgb,
    };
    let (w, h) = turned.dimensions();
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for pixel in turned.pixels() {
        let [r, g, b] = pixel.0;
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    Some((rgba, w, h))
}

/// How wide a picture is read at. A phone camera hands over twelve megapixels of
/// a screen, and neither the detection nor the reader wants them: reading at this
/// size costs milliseconds.
const READ_AT: u32 = 1600;

/// The name a QR carries and the seed inside it, as `(name, seed)`. The text goes
/// through the very parser a pasted one does, so a picture of a plain address is
/// refused in the same words a typed one would be.
fn decode(img: image::DynamicImage) -> Result<(Option<String>, String), String> {
    let working = if img.width().max(img.height()) > READ_AT {
        img.resize(READ_AT, READ_AT, image::imageops::FilterType::Gaussian)
    } else {
        img
    };
    let text = text_in(&working)?;
    totp::at(&text, now_secs()).map_err(|e| format!("that QR code is not a one-time seed: {e}"))?;
    Ok((totp::label(&text), text))
}

/// The one read: the picture in, the text out. ZXing's reader, because a
/// photograph of a screen is the hard case — finding the three corners is the
/// easy half, and the modules underneath carry the screen's own pixel grid, so
/// they arrive smudged however the threshold is set. This one corrects them;
/// finding a clean render is the smaller half of what it does.
///
/// The pixels reach it as ZXing's own luma: `306R + 601G + 117B`, the weights its
/// reference implementation carries. `image`'s `to_luma8` is Rec.709 and would
/// quietly change what the reader is looking at, which is the one thing this
/// path cannot afford — a phone's screenshot of a QR is already the hard case.
/// The buffer is built by hand and handed to `Luma8LuminanceSource`, so rxing's
/// `image` feature (and with it `imageproc` and every format `image` knows) never
/// enters the crate.
fn text_in(img: &image::DynamicImage) -> Result<String, String> {
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut luma = Vec::with_capacity((width as usize) * (height as usize));
    for pixel in rgba.pixels() {
        let [red, green, blue, alpha] = pixel.0;
        luma.push(if alpha == 0 {
            // Nothing there, so the reference reads it as white.
            0xFF
        } else {
            ((306 * (red as u64) + 601 * (green as u64) + 117 * (blue as u64) + 0x200) >> 10) as u8
        });
    }
    let source = rxing::Luma8LuminanceSource::new(luma, width, height).map_err(|e| {
        crate::shared::i18n::tf("that image cannot be read: {error}", &[("error", &e)])
    })?;
    let mut bitmap = rxing::BinaryBitmap::new(rxing::common::HybridBinarizer::new(source));
    let hints = rxing::DecodeHints {
        TryHarder: Some(true),
        PossibleFormats: Some(std::collections::HashSet::from([
            rxing::BarcodeFormat::QR_CODE,
        ])),
        ..Default::default()
    };
    match rxing::Reader::decode_with_hints(
        &mut rxing::MultiFormatReader::default(),
        &mut bitmap,
        &hints,
    ) {
        Ok(found) => Ok(found.getText().trim().to_owned()),
        Err(rxing::Exceptions::NotFoundException(_)) => {
            Err("no QR code found in that image".to_owned())
        }
        Err(e) => Err(format!("that QR code cannot be read: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::features::codes::model::Scan;

    #[test]
    fn a_new_code_lands_in_the_box_and_is_selected() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");

        app.boxes.add_otp();
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.otps.len(), 1);
        assert_eq!(s.selected.as_deref(), Some(s.vault.otps[0].id.as_str()));
        assert!(s.dirty, "a new code is a change to save");
    }

    /// A seed read out of a picture lands named, selected and marked to save.
    #[test]
    fn a_read_seed_lands_with_its_name() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");

        app.boxes
            .land_code(Some("Vercel".to_owned()), "JBSWY3DPEHPK3PXP".to_owned());
        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.otps.len(), 1);
        assert_eq!(s.vault.otps[0].name, "Vercel");
        assert_eq!(s.vault.otps[0].secret.as_str(), "JBSWY3DPEHPK3PXP");
        assert_eq!(s.selected.as_deref(), Some(s.vault.otps[0].id.as_str()));
        assert!(s.dirty);
    }

    /// A QR drawn by one crate and read by the other: the bytes a screenshot
    /// carries, and the seed a site would have shown. Two implementations, so
    /// the round trip proves the reader rather than a picture nobody can check.
    #[cfg(not(target_os = "android"))]
    fn qr(text: &str) -> Vec<u8> {
        const SCALE: u32 = 4;
        const QUIET: u32 = 4;
        let code = qrcode::QrCode::new(text.as_bytes()).unwrap();
        let side = code.width() as u32;
        let mut img = image::GrayImage::from_pixel(
            (side + QUIET * 2) * SCALE,
            (side + QUIET * 2) * SCALE,
            image::Luma([255]),
        );
        for (i, color) in code.to_colors().iter().enumerate() {
            if *color != qrcode::Color::Dark {
                continue;
            }
            let (x, y) = (i as u32 % side, i as u32 / side);
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    img.put_pixel(
                        (x + QUIET) * SCALE + dx,
                        (y + QUIET) * SCALE + dy,
                        image::Luma([0]),
                    );
                }
            }
        }
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        bytes
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_qr_carries_its_seed_and_its_name() {
        let uri = "otpauth://totp/Vercel:me@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Vercel";
        let png = qr(uri);
        let (name, seed) = super::decode(image::load_from_memory(&png).unwrap()).unwrap();
        assert_eq!(name.as_deref(), Some("Vercel"));
        assert_eq!(
            seed, uri,
            "the URI is kept whole, so its period and digits survive"
        );
    }

    /// The path a phone's picker and camera both answer with — and a desktop's
    /// dialog too: one reader for the three, and the user's own file is left
    /// where it was.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_file_on_disk_lands_as_a_named_code() {
        let ctx = egui::Context::default();
        let mut app = App::blank(&ctx);
        app.open("t");
        let path = std::env::temp_dir().join("pandora-qr-test.png");
        std::fs::write(
            &path,
            qr("otpauth://totp/Vercel?secret=JBSWY3DPEHPK3PXP&issuer=Vercel"),
        )
        .unwrap();

        let mut shell = crate::features::shell::model::Shell::default();
        app.boxes.land_file(&mut shell, &path);

        let s = app.boxes.session.as_ref().unwrap();
        assert_eq!(s.vault.otps.len(), 1);
        assert_eq!(s.vault.otps[0].name, "Vercel");
        assert!(
            path.exists(),
            "the file the user picked is not ours to remove"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A photograph of a screen, built here rather than checked in: the modules
    /// carry the screen's own pixel grid and the page a gradient, which is what
    /// left a real twelve-megapixel shot unreadable until the reader learned to
    /// average the grid away. The picture it came from is a phone's, and nobody
    /// else's, so no fixture — the two faults and the size are what matter.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_photo_of_a_screen_still_reads() {
        let uri =
            "otpauth://totp/ACME%20Co:jdoe@example.com?secret=JBSWY3DPEHPK3PXP&issuer=ACME%20Co";
        let grid = image::load_from_memory(&qr(uri)).unwrap().to_rgb8();
        let big = image::imageops::resize(&grid, 2400, 2400, image::imageops::FilterType::Triangle);
        let (w, h) = big.dimensions();
        let mut shot = image::RgbImage::new(w, h);
        for (x, y, px) in big.enumerate_pixels() {
            // Every third column is a shade off: the panel's own pixel grid.
            let stripe: i16 = if x % 3 == 0 { 12 } else { 0 };
            // And the page is not evenly lit: a lamp on one side.
            let fade: i16 = (x * 40 / w.max(1)) as i16 - 20;
            let shift = |c: u8| (c as i16 + stripe + fade).clamp(0, 255) as u8;
            let [r, g, b] = px.0;
            shot.put_pixel(x, y, image::Rgb([shift(r), shift(g), shift(b)]));
        }

        let (name, seed) = super::decode(image::DynamicImage::ImageRgb8(shot).blur(1.2)).unwrap();
        assert_eq!(name.as_deref(), Some("ACME Co"));
        assert_eq!(seed, uri);
    }

    /// What a camera answers with, and what it means.
    #[test]
    fn the_shape_a_camera_answers_is_read() {
        assert_eq!(super::shape_of("640x480 90"), Some((640, 480, 90)));
        assert_eq!(super::shape_of("  1280x720 0 "), Some((1280, 720, 0)));
        assert_eq!(super::shape_of("nonsense"), None);
        assert_eq!(super::shape_of("640x480"), None, "no turn, no frame");
    }

    /// A 4x4 frame in NV21, neutral chroma: the luma half-sized by its own
    /// subsampling, and turned to stand up.
    #[test]
    fn a_frame_reads_as_luma_and_stands_up_in_colour() {
        // luma 4x4, 1..16 row by row; chroma 2x2 pairs, all neutral
        let mut nv21: Vec<u8> = (1..=16u8).collect();
        nv21.extend([128u8; 8]);
        let flat = Scan::new(4, 4, 0);
        assert_eq!(
            super::luma(&nv21, &flat).unwrap().to_luma8().into_raw(),
            (1..=16u8).collect::<Vec<u8>>(),
            "the luma is the frame's first bytes, untouched, and full size"
        );

        let (rgba, w, h) = super::view(&nv21, &flat).unwrap();
        assert_eq!((w, h), (2, 2), "the preview is half the camera's size");
        assert_eq!(
            rgba,
            vec![3, 3, 3, 255, 5, 5, 5, 255, 11, 11, 11, 255, 13, 13, 13, 255],
            "four luma average into one pixel, and neutral chroma leaves it grey"
        );

        let (rgba, w, h) = super::view(&nv21, &Scan::new(4, 4, 90)).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            rgba,
            vec![11, 11, 11, 255, 3, 3, 3, 255, 13, 13, 13, 255, 5, 5, 5, 255],
            "a quarter turn takes the rows with it"
        );

        assert!(
            super::view(&[1, 2], &flat).is_none(),
            "a frame that is not the whole one is refused"
        );
    }

    /// A picture of something else is not a seed, and the refusal says so.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_qr_that_is_not_a_seed_is_refused() {
        let png = qr("https://example.com/not-a-seed");
        let err = super::decode(image::load_from_memory(&png).unwrap()).unwrap_err();
        assert!(err.contains("not a one-time seed"), "{err}");
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn an_image_with_no_qr_says_so() {
        let blank = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
            64,
            64,
            image::Luma([255]),
        ));
        let err = super::decode(blank).unwrap_err();
        assert!(err.contains("no QR code found"), "{err}");
    }

    /// What the clipboard gives back is pixels, not a file: the same QR, handed
    /// over as an RGBA buffer the way arboard does.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn the_same_qr_reads_from_pixels() {
        let uri = "otpauth://totp/Vercel?secret=JBSWY3DPEHPK3PXP&issuer=Vercel";
        let png = image::load_from_memory(&qr(uri)).unwrap().to_rgba8();
        let (width, height) = png.dimensions();
        let pixels = super::as_image(width, height, png.into_raw()).unwrap();
        let (name, seed) = super::decode(pixels).unwrap();
        assert_eq!(name.as_deref(), Some("Vercel"));
        assert_eq!(seed, uri);
    }

    /// Not a probe of the app but of the machine, and the one thing this side
    /// cannot know: whether a screenshot reaches the clipboard as a picture this
    /// app can read. Run it by hand —
    ///
    /// ```text
    /// cargo test --lib -- --ignored --nocapture probe_clipboard_image
    /// ```
    ///
    /// — after taking a screenshot over a QR. It says what it found, and never
    /// prints the seed.
    #[cfg(not(target_os = "android"))]
    #[test]
    #[ignore = "reads the real clipboard; run it by hand after a screenshot"]
    fn probe_clipboard_image() {
        let held = match super::clipboard_image() {
            Ok(Some(img)) => img,
            Ok(None) => {
                println!("PROBE: the clipboard holds no image");
                return;
            }
            Err(e) => {
                println!("PROBE: {e}");
                return;
            }
        };
        println!(
            "PROBE: the clipboard holds an image, {}x{}",
            held.width(),
            held.height()
        );
        match super::decode(held) {
            Ok((name, seed)) => println!(
                "PROBE: a one-time seed of {} chars, named {:?}",
                seed.chars().count(),
                name
            ),
            Err(e) => println!("PROBE: no seed in it — {e}"),
        }
    }
}
