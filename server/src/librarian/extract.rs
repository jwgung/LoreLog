//! The text inside a file that went through mail (#15): PDF, spreadsheets (xlsx, ods), Word (docx), PowerPoint (pptx),
//! HWP (5.0 and hwpx) and plain text. Pure Rust, no outside tools. Images are not read (no OCR), nor archives, nor the
//! old binary spreadsheets (xls, xlsb). A file that cannot be read gives no text; it never fails the index.
//!
//! The files come from anyone who mailed the user, so every reader is bounded: the file's size, the bytes unpacked
//! from it, the XML nodes, the text gathered. A reader runs in a thread of its own (a panic is caught, a slow one is
//! left), and only [`MAX_READERS`] at a time: a reader left running holds its place until it ends.
use std::io::{Cursor, Read};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use regex::Regex;

/// files bigger than this are not read
pub const MAX_FILE: usize = 30 * 1024 * 1024;
/// the text kept of one file
pub const MAX_TEXT: usize = 20_000;
/// one file may take this long; a reader that takes longer is left
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// readers running at once (a slow one left behind counts until it ends)
pub const MAX_READERS: usize = 2;
/// bumped when what is read changes: the cache of another version is not used
pub const VERSION: &str = "1";
/// what is unpacked of one part (an XML part of a zip, an HWP section), and of all of them
const MAX_PART: u64 = 8 * 1024 * 1024;
const MAX_UNPACKED: u64 = 64 * 1024 * 1024;
const MAX_PARTS: usize = 200;
const MAX_NODES: u32 = 1_000_000;
/// text gathered before it is tidied (more than is kept: whitespace goes)
const BUDGET: usize = MAX_TEXT * 4;

static READERS: AtomicUsize = AtomicUsize::new(0);

/// What reading a file gave.
#[derive(Debug, PartialEq)]
pub enum Read1 {
    Text(String),
    /// not a kind read, too big, broken, too slow, or nothing in it
    Nothing,
    /// every reader is busy: try again later (not to be remembered as nothing)
    Busy,
}

/// The kinds read, by file name.
pub fn readable(name: &str) -> bool {
    matches!(
        ext(name).as_str(),
        "pdf" | "xlsx" | "xlsm" | "ods" | "docx" | "pptx" | "hwp" | "hwpx" | "txt" | "csv" | "md" | "html" | "htm"
    )
}

pub fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.trim().to_lowercase()).unwrap_or_default()
}

/// The text of a file named `name`.
pub fn read_file(name: &str, bytes: Vec<u8>) -> Read1 {
    if !readable(name) || bytes.len() > MAX_FILE {
        return Read1::Nothing;
    }
    // wait for a place (a reader left behind may still hold one); none in time: busy
    let waited = std::time::Instant::now();
    while READERS.fetch_add(1, Ordering::SeqCst) >= MAX_READERS {
        READERS.fetch_sub(1, Ordering::SeqCst);
        if waited.elapsed() > TIMEOUT {
            return Read1::Busy;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let e = ext(name);
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("extract".into())
        .stack_size(64 * 1024 * 1024) // a deeply nested file must not overflow the stack (that is not a panic)
        .spawn(move || {
            let got = std::panic::catch_unwind(|| read(&e, &bytes)).ok().flatten();
            READERS.fetch_sub(1, Ordering::SeqCst);
            let _ = tx.send(got);
        });
    if spawned.is_err() {
        READERS.fetch_sub(1, Ordering::SeqCst);
        return Read1::Busy;
    }
    match rx.recv_timeout(TIMEOUT) {
        Ok(Some(raw)) => {
            let text = tidy(&raw);
            if text.is_empty() { Read1::Nothing } else { Read1::Text(text) }
        }
        _ => Read1::Nothing,
    }
}

/// [`read_file`] as an option (busy counts as nothing).
pub fn text_of(name: &str, bytes: &[u8]) -> Option<String> {
    match read_file(name, bytes.to_vec()) {
        Read1::Text(t) => Some(t),
        _ => None,
    }
}

fn read(ext: &str, data: &[u8]) -> Option<String> {
    match ext {
        "pdf" => pdf_extract::extract_text_from_mem(data).ok(),
        // the words of a workbook are its shared strings (and strings written in a cell)
        "xlsx" | "xlsm" => xml_parts(data, |n| {
            n == "xl/sharedStrings.xml" || (n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml"))
        }),
        "ods" => xml_parts(data, |n| n == "content.xml"),
        "docx" => xml_parts(data, |n| n == "word/document.xml"),
        "pptx" => xml_parts(data, |n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml")),
        "hwpx" => xml_parts(data, |n| n.starts_with("Contents/section") && n.ends_with(".xml")),
        "hwp" => hwp(data),
        "html" | "htm" => Some(html(&String::from_utf8_lossy(data))),
        _ => Some(String::from_utf8_lossy(&data[..data.len().min(BUDGET * 4)]).into_owned()),
    }
}

/// Whitespace made single, control letters dropped, cut to [`MAX_TEXT`].
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let (mut n, mut gap) = (0, false);
    for c in text.chars() {
        if c.is_whitespace() {
            gap = !out.is_empty();
            continue;
        }
        if c.is_control() {
            continue;
        }
        if gap {
            out.push(' ');
            n += 1;
            gap = false;
        }
        if n >= MAX_TEXT {
            break;
        }
        out.push(c);
        n += 1;
    }
    out.chars().take(MAX_TEXT).collect::<String>().trim_end().to_string()
}

fn html(text: &str) -> String {
    static TAG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<script\b.*?</script>|<style\b.*?</style>|<[^>]*>").unwrap());
    html_escape::decode_html_entities(&TAG.replace_all(&text[..text.floor_char_boundary(BUDGET * 4)], " ")).into_owned()
}

/// The text of the XML parts of a zip (Office Open XML, OpenDocument, HWPX) whose names `want`, in name order (slide2
/// before slide10): the text of `t` elements (`w:t`, `a:t`, `hp:t`, a shared string) and of OpenDocument paragraphs
/// (`text:p`, `text:span`); a paragraph a line.
fn xml_parts(data: &[u8], want: impl Fn(&str) -> bool) -> Option<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data)).ok()?;
    let mut names: Vec<String> =
        zip.file_names().filter_map(|n| n.ok()).map(|n| n.to_string()).filter(|n| want(n)).collect();
    let num = |n: &str| -> u32 { n.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0) };
    names.sort_by_key(|n| (num(n), n.clone()));
    names.truncate(MAX_PARTS);
    let (mut out, mut unpacked) = (String::new(), 0u64);
    for n in names {
        if out.len() > BUDGET || unpacked > MAX_UNPACKED {
            break;
        }
        let mut xml = String::new();
        let Ok(f) = zip.by_name(&n) else { continue };
        if f.take(MAX_PART).read_to_string(&mut xml).is_err() {
            continue;
        }
        unpacked += xml.len() as u64;
        let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES, ..Default::default() };
        let Ok(doc) = roxmltree::Document::parse_with_options(&xml, opts) else { continue };
        for node in doc.descendants() {
            if out.len() > BUDGET {
                break;
            }
            if node.is_text() {
                let parent = node.parent().map(|p| p.tag_name().name()).unwrap_or("");
                if matches!(parent, "t" | "p" | "span" | "h")
                    && let Some(t) = node.text()
                {
                    out.push_str(t);
                }
            } else if matches!(node.tag_name().name(), "p" | "si" | "c") {
                // a paragraph, a shared string, a cell: apart from the next
                out.push('\n');
            }
        }
        out.push('\n');
    }
    Some(out)
}

/// HWP 5.0: the paragraph text records of the body sections (compressed or not); a document with a password gives
/// only its preview text (`PrvText`), which is also what a section that cannot be read falls back to.
fn hwp(data: &[u8]) -> Option<String> {
    let mut cf = cfb::CompoundFile::open(Cursor::new(data)).ok()?;
    let mut header = Vec::new();
    cf.open_stream("/FileHeader").ok()?.take(4096).read_to_end(&mut header).ok()?;
    let flags = header.get(36..40).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
    let (compressed, locked) = (flags & 1 != 0, flags & 2 != 0);
    let (mut out, mut unpacked) = (String::new(), 0u64);
    if !locked {
        for i in 0..MAX_PARTS {
            if out.len() > BUDGET || unpacked > MAX_UNPACKED {
                break;
            }
            let Ok(s) = cf.open_stream(format!("/BodyText/Section{i}")) else { break };
            let mut raw = Vec::new();
            if s.take(MAX_PART).read_to_end(&mut raw).is_err() {
                break;
            }
            let body = if compressed {
                let mut d = Vec::new();
                if flate2::read::DeflateDecoder::new(&raw[..]).take(MAX_PART).read_to_end(&mut d).is_err() {
                    break;
                }
                d
            } else {
                raw
            };
            unpacked += body.len() as u64;
            out.push_str(&para_texts(&body));
        }
    }
    if out.trim().is_empty() {
        let mut prv = Vec::new();
        cf.open_stream("/PrvText").ok()?.take(MAX_PART).read_to_end(&mut prv).ok()?;
        out = utf16(&prv);
    }
    Some(out)
}

/// HWPTAG_PARA_TEXT (HWPTAG_BEGIN 16 + 51)
const PARA_TEXT: u32 = 67;

/// The text of the paragraph records of a section: UTF-16LE, its controls left out (a control of the extended and
/// inline kinds takes 8 letters; a line break, a paragraph end and the others take one).
fn para_texts(body: &[u8]) -> String {
    let mut out = String::new();
    let mut at = 0;
    let u32_at = |i: usize| body.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    while let Some(h) = u32_at(at) {
        let tag = h & 0x3FF;
        let mut size = (h >> 20) as usize;
        at += 4;
        if size == 0xFFF {
            let Some(s) = u32_at(at) else { break };
            size = s as usize;
            at += 4;
        }
        let Some(rec) = body.get(at..at + size) else { break };
        at += size;
        if tag != PARA_TEXT {
            continue;
        }
        let units: Vec<u16> = rec.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let mut i = 0;
        let mut text = Vec::new();
        while i < units.len() {
            let u = units[i];
            match u {
                0..=31 => {
                    if matches!(u, 10 | 13) {
                        text.push(b'\n' as u16);
                    }
                    i += if matches!(u, 0 | 10 | 13 | 24..=31) { 1 } else { 8 };
                }
                _ => {
                    text.push(u);
                    i += 1;
                }
            }
        }
        out.push_str(&String::from_utf16_lossy(&text));
        out.push('\n');
        if out.len() > BUDGET {
            break;
        }
    }
    out
}

fn utf16(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in parts {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn word_powerpoint_and_hwpx_give_their_paragraphs() {
        let docx = zip_of(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>납품 일정</w:t></w:r><w:r><w:t>확정</w:t></w:r></w:p><w:p><w:r><w:t>둘째 문단</w:t></w:r></w:p></w:body></w:document>"#,
        )]);
        assert_eq!(text_of("계약.docx", &docx).unwrap(), "납품 일정확정 둘째 문단");
        let slide = |t: &str| format!(r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>{t}</a:t></a:r></a:p></p:sld>"#);
        let pptx = zip_of(&[("ppt/slides/slide10.xml", &slide("열째")), ("ppt/slides/slide2.xml", &slide("둘째"))]);
        assert_eq!(text_of("발표.PPTX", &pptx).unwrap(), "둘째 열째", "slides in their order");
        let hwpx = zip_of(&[(
            "Contents/section0.xml",
            r#"<hs:sec xmlns:hs="s" xmlns:hp="p"><hp:p><hp:run><hp:t>견적 금액</hp:t></hp:run></hp:p></hs:sec>"#,
        )]);
        assert_eq!(text_of("견적.hwpx", &hwpx).unwrap(), "견적 금액");
    }

    fn hwp_file(compressed: bool, text: &str) -> Vec<u8> {
        let mut cf = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        let mut header = vec![0u8; 256];
        header[..17].copy_from_slice(b"HWP Document File");
        header[36] = compressed as u8;
        cf.create_stream("/FileHeader").unwrap().write_all(&header).unwrap();
        // a paragraph: a section-definition control (8 letters), the text, the paragraph end
        let mut units: Vec<u16> = vec![2, 0x6463, 0x6573, 0, 0, 0, 0, 2];
        units.extend(text.encode_utf16());
        units.push(13);
        let rec: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        let mut body = (PARA_TEXT | ((rec.len() as u32) << 20)).to_le_bytes().to_vec();
        body.extend(&rec);
        let other = 66u32 | (4 << 20); // a paragraph header record: not text
        body.splice(0..0, other.to_le_bytes().iter().copied().chain([1, 2, 3, 4]));
        let body = if compressed {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(&body).unwrap();
            e.finish().unwrap()
        } else {
            body
        };
        cf.create_storage("/BodyText").unwrap();
        cf.create_stream("/BodyText/Section0").unwrap().write_all(&body).unwrap();
        cf.flush().unwrap();
        cf.into_inner().into_inner()
    }

    #[test]
    fn hwp_gives_its_paragraph_text_without_its_controls() {
        assert_eq!(text_of("보고.hwp", &hwp_file(false, "월간 보고서")).unwrap(), "월간 보고서");
        assert_eq!(text_of("보고.hwp", &hwp_file(true, "압축된 본문")).unwrap(), "압축된 본문");
    }

    #[test]
    fn a_pdf_gives_its_text() {
        let content = "BT /F1 12 Tf 72 712 Td (Quotation PO-7731) Tj ET";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            pdf.extend(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes(),
        );
        assert_eq!(text_of("견적.pdf", &pdf).unwrap(), "Quotation PO-7731");
    }

    #[test]
    fn what_is_not_read_gives_nothing_and_a_broken_file_does_not_break_anything() {
        assert!(text_of("사진.jpg", b"\xff\xd8\xff").is_none());
        assert!(text_of("묶음.zip", &zip_of(&[("a.txt", "x")])).is_none());
        assert!(text_of("깨진.pdf", b"%PDF-1.4 garbage").is_none());
        assert!(text_of("깨진.xlsx", b"PK\x03\x04 nope").is_none());
        assert!(text_of("깨진.hwp", b"not ole").is_none());
        assert!(text_of("큰.txt", &vec![b'a'; MAX_FILE + 1]).is_none());
        assert_eq!(text_of("메모.txt", "  한 줄\n\n둘째 줄 ".as_bytes()).unwrap(), "한 줄 둘째 줄");
        assert_eq!(text_of("p.html", b"<style>x{}</style><p>Hello &amp; bye</p>").unwrap(), "Hello & bye");
        assert_eq!(text_of("긴.txt", "가".repeat(MAX_TEXT + 10).as_bytes()).unwrap().chars().count(), MAX_TEXT);
    }
}

#[cfg(test)]
mod bounds {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_workbook_gives_its_strings_and_a_huge_one_is_bounded() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("xl/sharedStrings.xml", opts).unwrap();
        w.write_all(r#"<sst xmlns="x"><si><t>품목</t></si><si><t>엑셀속문구고슴도치</t></si></sst>"#.as_bytes())
            .unwrap();
        // a sheet that unpacks to far more than is read: one cell at the far corner, and a long run of empty rows
        w.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
        w.write_all(b"<worksheet><sheetData>").unwrap();
        for _ in 0..200_000 {
            w.write_all(b"<row><c r=\"XFD1048576\"><is><t>x</t></is></c></row>").unwrap();
        }
        w.write_all(b"</sheetData></worksheet>").unwrap();
        let xlsx = w.finish().unwrap().into_inner();
        let t = text_of("표.xlsx", &xlsx).unwrap();
        assert!(t.starts_with("품목 엑셀속문구고슴도치"), "{}", &t[..40.min(t.len())]);
        assert!(t.chars().count() <= MAX_TEXT);
    }
}
