//! Connecting a phone on the first run (#69): the connection code drawn as a QR picture, made on this PC (nothing is
//! fetched), and opening the Tailscale download page in the user's browser.

/// The text as a QR code, an SVG picture (black on white, with the quiet zone readers need).
pub fn qr_svg(text: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| format!("QR을 만들지 못했습니다: {e}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build())
}

pub const TAILSCALE_DOWNLOAD: &str = "https://tailscale.com/download";

/// Open a fixed address in the default browser (the app's own window only shows its bundled pages).
pub fn open_in_browser(url: &'static str) -> Result<(), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| format!("브라우저를 열지 못했습니다({e}). {url} 를 직접 열어 주세요"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_code_fits_one_qr_picture() {
        let code = format!("lorelog://connect?u=http%3A%2F%2Fpc.tail8d1ab7.ts.net%3A8765&t={}", "0".repeat(32));
        let svg = qr_svg(&code).unwrap();
        assert!(svg.contains("<svg") && svg.trim_end().ends_with("</svg>"), "{svg}");
        assert!(svg.contains("#000000") && svg.contains("#ffffff"));
        assert!(!svg.contains("<script"), "only shapes");
        // the same text, the same picture
        assert_eq!(svg, qr_svg(&code).unwrap());
    }
}
