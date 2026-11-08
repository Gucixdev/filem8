//! File classification by extension (and a few magic bytes). One table so
//! the icon, the preview chooser and the "type" sort column agree.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    Dir,
    Image,
    Audio,
    Video,
    Text,
    Code,
    Archive,
    Pdf,
    Model3d,
    Document,
    Data,
    Font,
    Other,
}

const IMAGE: &[&str] = &["png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "avif", "bmp", "tif", "tiff", "svg", "ico", "tga", "qoi", "pnm", "ppm", "pgm", "pbm", "pam", "hdr", "exr", "dds", "ff", "heic", "heif", "jxl", "apng", "xpm", "psd", "xcf", "kra", "ora"];
const AUDIO: &[&str] = &["mp3", "flac", "ogg", "oga", "opus", "wav", "aac", "m4a", "wma", "aiff", "aif", "mid", "midi", "mka", "ape", "wv", "mod", "xm", "it", "s3m"];
const VIDEO: &[&str] = &["mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "mpg", "mpeg", "ogv", "ts", "3gp"];
const TEXT: &[&str] = &["txt", "md", "markdown", "rst", "log", "nfo", "csv", "tsv", "ini", "conf", "cfg", "toml", "yaml", "yml", "json", "xml", "html", "htm", "css", "tex", "org", "adoc", "srt", "vtt", "desktop", "service", "theme", "env", "lock", "gitignore", "editorconfig"];
const CODE: &[&str] = &["rs", "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "py", "js", "mjs", "ts_", "tsx", "jsx", "go", "java", "kt", "swift", "rb", "php", "lua", "sh", "bash", "zsh", "fish", "pl", "pm", "r", "cs", "fs", "hs", "ml", "ex", "exs", "erl", "clj", "scala", "dart", "zig", "nim", "v", "vhd", "sql", "gd", "gdshader", "glsl", "wgsl", "vert", "frag", "hlsl", "mojo", "el", "vim", "mk", "cmake", "gradle", "tscn", "tres", "godot", "lean", "s", "asm"];
const ARCHIVE: &[&str] = &["zip", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "7z", "rar", "lz4", "lzma", "jar", "apk", "deb", "rpm", "iso", "cab", "cpio", "pkg", "whl", "crate", "epub_"];
const MODEL: &[&str] = &["gltf", "glb", "obj", "stl", "ply", "fbx", "dae", "blend", "3ds", "off", "usd", "usdz"];
const DOC: &[&str] = &["doc", "docx", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "rtf", "epub", "mobi", "djvu", "ps"];
const DATA: &[&str] = &["db", "sqlite", "sqlite3", "bin", "dat", "pak", "wad", "gguf", "safetensors", "pt", "onnx", "npy", "npz", "pickle", "pkl", "tmx", "tsx_", "ldtk", "aseprite", "ase"];
const FONT: &[&str] = &["ttf", "otf", "woff", "woff2", "ttc", "pcf", "bdf"];

pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(0) | None => String::new(),
        Some(i) => name[i + 1..].to_ascii_lowercase(),
    }
}

pub fn classify_name(name: &str, is_dir: bool) -> Class {
    if is_dir {
        return Class::Dir;
    }
    let e = ext_of(name);
    let e = e.as_str();
    if e == "pdf" {
        Class::Pdf
    } else if IMAGE.contains(&e) {
        Class::Image
    } else if AUDIO.contains(&e) {
        Class::Audio
    } else if VIDEO.contains(&e) {
        Class::Video
    } else if MODEL.contains(&e) {
        Class::Model3d
    } else if ARCHIVE.contains(&e) {
        Class::Archive
    } else if CODE.contains(&e) {
        Class::Code
    } else if TEXT.contains(&e) {
        Class::Text
    } else if DOC.contains(&e) {
        Class::Document
    } else if FONT.contains(&e) {
        Class::Font
    } else if DATA.contains(&e) {
        Class::Data
    } else {
        match name {
            "Makefile" | "makefile" | "Dockerfile" | "PKGBUILD" | "LICENSE" | "README" | "CMakeLists.txt" | "Cargo.toml" => Class::Text,
            _ => Class::Other,
        }
    }
}

/// Sniff the first bytes; used when the extension says nothing.
pub fn sniff(head: &[u8]) -> Class {
    let h = head;
    if h.starts_with(b"\x89PNG") || h.starts_with(b"\xff\xd8\xff") || h.starts_with(b"GIF8") || h.starts_with(b"BM") || (h.len() >= 12 && &h[0..4] == b"RIFF" && &h[8..12] == b"WEBP") || (h.len() >= 12 && &h[4..8] == b"ftyp" && (&h[8..12] == b"avif" || &h[8..12] == b"avis")) {
        Class::Image
    } else if h.starts_with(b"%PDF") {
        Class::Pdf
    } else if h.starts_with(b"PK\x03\x04") || h.starts_with(b"\x1f\x8b") || h.starts_with(b"7z\xbc\xaf") || h.starts_with(b"BZh") {
        Class::Archive
    } else if h.starts_with(b"ID3") || h.starts_with(b"fLaC") || h.starts_with(b"OggS") || (h.len() >= 12 && &h[0..4] == b"RIFF" && &h[8..12] == b"WAVE") {
        Class::Audio
    } else if h.starts_with(b"glTF") {
        Class::Model3d
    } else if !h.is_empty() && !h.contains(&0) && std::str::from_utf8(h).is_ok() {
        Class::Text
    } else {
        Class::Other
    }
}

pub fn class_label(c: Class) -> &'static str {
    match c {
        Class::Dir => "Folder",
        Class::Image => "Image",
        Class::Audio => "Audio",
        Class::Video => "Video",
        Class::Text => "Text",
        Class::Code => "Source code",
        Class::Archive => "Archive",
        Class::Pdf => "PDF",
        Class::Model3d => "3D model",
        Class::Document => "Document",
        Class::Data => "Data",
        Class::Font => "Font",
        Class::Other => "File",
    }
}

/// MIME type guessed from the extension (small table; "application/octet-stream" otherwise).
pub fn mime_of(name: &str, is_dir: bool) -> &'static str {
    if is_dir { return "inode/directory"; }
    match ext_of(name).as_str() {
        "png" => "image/png", "jpg" | "jpeg" | "jpe" | "jfif" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp",
        "avif" => "image/avif", "bmp" => "image/bmp", "svg" => "image/svg+xml", "ico" => "image/x-icon", "tif" | "tiff" => "image/tiff",
        "heic" | "heif" => "image/heic", "jxl" => "image/jxl", "qoi" => "image/x-qoi",
        "mp3" => "audio/mpeg", "flac" => "audio/flac", "ogg" | "oga" | "opus" => "audio/ogg", "wav" => "audio/wav",
        "m4a" | "aac" => "audio/mp4", "mid" | "midi" => "audio/midi",
        "mp4" | "m4v" => "video/mp4", "mkv" => "video/x-matroska", "webm" => "video/webm", "avi" => "video/x-msvideo", "mov" => "video/quicktime",
        "txt" | "log" => "text/plain", "md" | "markdown" => "text/markdown", "html" | "htm" => "text/html", "css" => "text/css",
        "csv" => "text/csv", "json" => "application/json", "xml" => "application/xml", "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml", "js" | "mjs" => "text/javascript", "rs" => "text/x-rust", "c" | "h" => "text/x-c",
        "cpp" | "cc" | "hpp" => "text/x-c++", "py" => "text/x-python", "sh" | "bash" => "application/x-shellscript",
        "pdf" => "application/pdf", "zip" => "application/zip", "gz" | "tgz" => "application/gzip", "tar" => "application/x-tar",
        "xz" => "application/x-xz", "zst" => "application/zstd", "7z" => "application/x-7z-compressed", "rar" => "application/vnd.rar",
        "bz2" => "application/x-bzip2", "iso" => "application/x-iso9660-image",
        "gltf" => "model/gltf+json", "glb" => "model/gltf-binary", "obj" => "model/obj", "stl" => "model/stl",
        "ttf" => "font/ttf", "otf" => "font/otf", "woff" => "font/woff", "woff2" => "font/woff2",
        "doc" | "docx" => "application/msword", "odt" => "application/vnd.oasis.opendocument.text",
        "xls" | "xlsx" => "application/vnd.ms-excel", "epub" => "application/epub+zip",
        _ => "application/octet-stream",
    }
}
