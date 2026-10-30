//! The diary assistant: what it reads about a day, the day's note, the senders the user allowed, the chat and to-dos.
pub mod chat;
pub mod context;
pub mod diary;
pub mod senders;
pub mod util;

/// Phone system apps whose notifications and app events say nothing about the user's day.
pub const NOISE_PKGS: [&str; 3] = [
    "com.android.systemui",
    "com.sec.android.app.launcher",
    "com.samsung.android.honeyboard",
];
