#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Alsa(alsa::Error),
    Opus(opus::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Alsa(e) => std::fmt::Display::fmt(e, f),
            Error::Opus(e) => std::fmt::Display::fmt(e, f),
        }
    }
}

impl From<alsa::Error> for Error {
    fn from(value: alsa::Error) -> Self {
        Self::Alsa(value)
    }
}

impl From<opus::Error> for Error {
    fn from(value: opus::Error) -> Self {
        Self::Opus(value)
    }
}

impl std::error::Error for Error {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Error::Alsa(x) => Some(x),
            Error::Opus(x) => Some(x),
        }
    }
}
