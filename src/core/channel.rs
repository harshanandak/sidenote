#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Stable,
    Dev,
    Latest,
}

pub const CHANNEL: Channel = if cfg!(feature = "dev") {
    Channel::Dev
} else {
    Channel::Stable
};

impl Channel {
    /// The update feed can differ from the build channel used for feature gates.
    pub const DEFAULT_UPDATE: Self = match option_env!("SIDENOTE_DEFAULT_CHANNEL") {
        Some(value) if matches!(value.as_bytes(), b"latest") => Self::Latest,
        _ => CHANNEL,
    };
}
