//! Line icons on a 16-point grid, stroked in the current colour.

use rune::Icon;

macro_rules! icon {
    ($id:ident, $name:literal, $body:literal) => {
        pub static $id: Icon = Icon { name: concat!("files-", $name), view: 16.0, body: $body };
    };
}

icon!(
    BACK,
    "back",
    r#"<path d="M12.5 8H3.5M7 4.5L3.5 8L7 11.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    FORWARD,
    "forward",
    r#"<path d="M3.5 8H12.5M9 4.5L12.5 8L9 11.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    UP,
    "up",
    r#"<path d="M8 12.5V3.5M4.5 7L8 3.5L11.5 7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    REFRESH,
    "refresh",
    r#"<path d="M12.6 8.4A4.6 4.6 0 1 1 11.3 4.7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/><path d="M12.4 2.6V5.3H9.7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(HOME, "home", r#"<path d="M3 7L8 3L13 7V12.5H9.5V9.5H6.5V12.5H3Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#);
icon!(
    DESKTOP,
    "desktop",
    r#"<rect x="2.5" y="3" width="11" height="7.5" rx="1.2" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M6 13H10M8 10.5V13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#
);
icon!(
    DOCUMENTS,
    "documents",
    r#"<path d="M4 2.5H9L12 5.5V13.5H4Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M6 8H10M6 10.5H10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#
);
icon!(
    DOWNLOADS,
    "downloads",
    r#"<path d="M8 2.5V10M4.8 7L8 10.2L11.2 7" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M3 13H13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#
);
icon!(
    UPLOAD,
    "upload",
    r#"<path d="M8 10.5V3M4.8 6L8 2.8L11.2 6" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M3 13H13" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#
);
icon!(
    PICTURES,
    "pictures",
    r#"<rect x="2.5" y="3" width="11" height="10" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="6" cy="6.5" r="1.1" fill="currentColor"/><path d="M3 11.5L6.5 8.5L9 10.5L10.5 9.3L13 11.3" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    MUSIC,
    "music",
    r#"<path d="M6.5 11.5V3.8L12.5 2.5V10.2" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><circle cx="5" cy="11.6" r="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11" cy="10.3" r="1.6" stroke="currentColor" stroke-width="1.2" fill="none"/>"#
);
icon!(
    VIDEOS,
    "videos",
    r#"<rect x="2.5" y="4" width="8" height="8" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M10.5 7L13.5 5.2V10.8L10.5 9" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    DRIVE,
    "drive",
    r#"<rect x="2.5" y="4.5" width="11" height="7" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="11" cy="8" r=".9" fill="currentColor"/><path d="M4.5 8H8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#
);
icon!(
    SERVER,
    "server",
    r#"<rect x="2.5" y="2.8" width="11" height="4.4" rx="1.2" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="2.5" y="8.8" width="11" height="4.4" rx="1.2" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="5" cy="5" r=".8" fill="currentColor"/><circle cx="5" cy="11" r=".8" fill="currentColor"/>"#
);
icon!(
    CONNECT,
    "connect",
    r#"<rect x="2.5" y="2.8" width="11" height="4.4" rx="1.2" stroke="currentColor" stroke-width="1.2" fill="none"/><circle cx="5" cy="5" r=".8" fill="currentColor"/><path d="M8 9.5V13.5M6 11.5H10" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#
);
icon!(
    FOLDER_PLUS,
    "folder-plus",
    r#"<path d="M2.5 4.5C2.5 3.9 2.9 3.5 3.5 3.5H6.3L7.6 5H12.5C13.1 5 13.5 5.4 13.5 6V11.5C13.5 12.1 13.1 12.5 12.5 12.5H3.5C2.9 12.5 2.5 12.1 2.5 11.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M8 7V11M6 9H10" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#
);
icon!(
    TRANSFERS,
    "transfers",
    r#"<path d="M5 13V3M2.5 5.5L5 3L7.5 5.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M11 3V13M8.5 10.5L11 13L13.5 10.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    TRASH,
    "trash",
    r#"<path d="M3 4.5H13M6.5 4.5V3H9.5V4.5M4.5 4.5L5.2 13H10.8L11.5 4.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    LINK,
    "link",
    r#"<path d="M7 9L9 7M6 7.5L4.6 8.9A2 2 0 0 0 7.1 11.4L8.5 10M10 8.5L11.4 7.1A2 2 0 0 0 8.9 4.6L7.5 6" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#
);
icon!(
    ARCHIVE,
    "archive",
    r#"<rect x="2.5" y="3" width="11" height="3" rx=".8" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M3.5 6V12.5H12.5V6M6.5 8.5H9.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(
    TERMINAL,
    "terminal",
    r#"<rect x="2" y="3" width="12" height="10" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M4.8 6.3L6.8 8L4.8 9.7M8.3 10H11" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
icon!(DOT, "dot", r#"<circle cx="8" cy="8" r="3" fill="currentColor"/>"#);
icon!(PLUS, "plus", r#"<path d="M8 3.5V12.5M3.5 8H12.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#);
icon!(
    FOLDER_GO,
    "folder-go",
    r#"<path d="M2.5 4.5C2.5 3.9 2.9 3.5 3.5 3.5H6.3L7.6 5H12.5C13.1 5 13.5 5.4 13.5 6V11.5C13.5 12.1 13.1 12.5 12.5 12.5H3.5C2.9 12.5 2.5 12.1 2.5 11.5Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M6 9H10M8.5 7.5L10 9L8.5 10.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#
);
