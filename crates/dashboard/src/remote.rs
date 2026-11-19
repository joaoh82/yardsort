//! Remote from the page: signing this computer in to a Yardsort server, and letting devices in.
//!
//! The program serving the dashboard does these itself, through [`Remote`]. `yardsort-host` does;
//! the app keeps them in its own Settings, and gives none.
//!
//! **This is the one place a signed-in browser does what the operator's guard otherwise keeps
//! for a person at a terminal.** A device is let in here by whoever holds a dashboard session,
//! on purpose: a server set up from a browser is finished from one. So the dashboard's sign-in
//! is part of what guards pairing. A command or a terminal the dashboard runs is still refused
//! `yardsort-host pair`: only the page's own asking is taken, never a program's.

use serde::{Deserialize, Serialize};

/// Everything here may block: it is called off the async runtime.
pub trait Remote: Send + Sync + 'static {
    /// The devices let in here, and those asking to be.
    fn devices(&self) -> Result<Devices, String>;
    /// A pairing code for a device to type or scan, good once and for a few minutes.
    fn offer(&self) -> Result<Offer, String>;
    /// Answer a device that asked.
    fn answer(&self, grant: &str, answer: Answer) -> Result<(), String>;
    /// Take a device's way in away. Its name.
    fn revoke(&self, grant: &str) -> Result<String, String>;
    /// Start signing this computer in at `server`, to be called `name` there: the address to
    /// sign in at, in any browser.
    fn begin_sign_in(&self, server: &str, name: &str) -> Result<SigningIn, String>;
    /// The sign-in under way, if one is, or why the last one came to nothing.
    fn signing_in(&self) -> SignInState;
    /// Finish it with the address the browser was sent back to.
    fn finish_sign_in(&self, address: &str) -> Result<(), String>;
    /// Give the sign-in under way up.
    fn cancel_sign_in(&self);
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Devices {
    pub devices: Vec<Device>,
    pub asking: Vec<Asking>,
    /// The projects a device can be kept to, when there is more than one.
    pub projects: Vec<Choice>,
}

/// A device let in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub grant: String,
    pub name: String,
    /// It may type into terminals, not only watch.
    pub control: bool,
    /// The projects it is kept to, by name. All of them when absent.
    pub projects: Option<Vec<String>>,
}

/// A device asking to be let in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asking {
    pub grant: String,
    pub name: String,
    pub wants_control: bool,
    pub wants_workflows: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Offer {
    /// To type into the device.
    pub code: String,
    /// To scan with it: `yardsort://pair?…`. Holds the offer's secret.
    pub link: String,
    pub expires_in_ms: u32,
}

/// What the person at the page chose for a device that asked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub allow: bool,
    /// Typing into terminals too.
    #[serde(default)]
    pub control: bool,
    /// Approving or answering workflows too.
    #[serde(default)]
    pub workflows: bool,
    /// Only these projects, by id. All of them when absent.
    #[serde(default)]
    pub projects: Option<Vec<String>>,
}

/// A sign-in under way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SigningIn {
    /// Where to sign in, in a browser on any computer.
    pub url: String,
    /// The port on this computer the browser is sent back to.
    pub port: u16,
    pub server: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SignInState {
    #[default]
    None,
    Waiting(SigningIn),
    /// The last one came to nothing: why.
    Failed {
        message: String,
    },
}

/// A link as the dots of a QR code, a row a line, `#` for dark: drawn by the page.
pub fn qr(link: &str) -> Vec<String> {
    let Ok(code) = qrcode::QrCode::new(link.as_bytes()) else {
        return Vec::new();
    };
    let width = code.width();
    code.to_colors()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|dot| match dot {
                    qrcode::Color::Dark => '#',
                    qrcode::Color::Light => '.',
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_drawn_as_a_square_of_dots_with_its_three_corners() {
        let rows = qr("yardsort://pair?server=https%3A%2F%2Fexample.test&secret=abc");
        assert!(rows.len() >= 21);
        assert!(rows.iter().all(|row| row.len() == rows.len()));
        // The finder pattern: a dark bar seven wide at the top left and the top right.
        assert!(rows[0].starts_with("#######."), "{}", rows[0]);
        assert!(rows[0].ends_with(".#######"), "{}", rows[0]);
        assert!(rows[rows.len() - 1].starts_with("#######."));
    }
}
