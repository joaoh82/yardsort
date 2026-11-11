//! `yardsort-host pair`: the operator's side. Shows the code and a QR code, and when a device asks,
//! asks the operator what to let it do. Nothing is decided without their answer.

use std::io::{IsTerminal, Write};

use tokio::io::{AsyncBufReadExt, BufReader, Lines, Stdin};

use crate::control::{self, Ask, Project, Reply};
use crate::profile::Profile;

pub async fn run(profile: &Profile) -> Result<(), String> {
    // The host checks this itself, by what the system says of this process; said here first only
    // so the reason is the plainest one.
    if !std::io::stdin().is_terminal() {
        return Err(
            "Pairing needs a person at a terminal: run `yardsort-host pair` in one.".into(),
        );
    }
    let mut line = control::connect(profile).await?;
    line.send(&Ask::Pair).await?;
    let mut typed = BufReader::new(tokio::io::stdin()).lines();
    loop {
        let Some(reply) = line.next::<Reply>().await? else {
            return Err("The host closed the connection.".into());
        };
        match reply {
            Reply::Offer {
                code,
                link,
                expires_in_ms,
            } => {
                println!("{}", qr(&link));
                println!("Scan this with the Yardsort app, or type this code in it:\n");
                println!("  {code}\n");
                println!(
                    "Good once, for {} minutes. Waiting for a device to ask… (Ctrl-C to stop)",
                    expires_in_ms.div_ceil(60_000)
                );
            }
            Reply::Asked {
                grant,
                device,
                wants_control,
                projects,
            } => {
                println!(
                    "\n{device} asks to {}.",
                    if wants_control {
                        "watch and type into this computer's terminals"
                    } else {
                        "watch this computer's terminals"
                    }
                );
                println!("Watching includes reading workflow runs.");
                let (allow, control) = choose(&mut typed, wants_control).await?;
                let workflows = if allow {
                    matches!(
                        ask(
                            &mut typed,
                            "Also let it approve or answer workflows? [y/N] "
                        )
                        .await?
                        .as_str(),
                        "y" | "yes"
                    )
                } else {
                    false
                };
                let projects = if allow {
                    which_projects(&mut typed, &projects).await?
                } else {
                    None
                };
                line.send(&Ask::Answer {
                    grant,
                    allow,
                    control,
                    workflows,
                    projects,
                })
                .await?;
            }
            Reply::Answered { allowed: true } => {
                println!("Let in. It reaches this computer now.");
                return Ok(());
            }
            Reply::Answered { allowed: false } => {
                println!("Refused. Nothing was let in.");
                return Ok(());
            }
            Reply::Lapsed => return Err("The code lapsed with no device asking.".into()),
            Reply::Failed { message } => return Err(message),
            _ => return Err("The host answered something else.".into()),
        }
    }
}

async fn ask(typed: &mut Lines<BufReader<Stdin>>, question: &str) -> Result<String, String> {
    print!("{question}");
    let _ = std::io::stdout().flush();
    match typed.next_line().await {
        Ok(Some(answer)) => Ok(answer.trim().to_lowercase()),
        _ => Err("Stopped without an answer. Nothing was let in.".into()),
    }
}

/// Allow viewing, viewing and typing, or deny: `(allow, control)`.
async fn choose(
    typed: &mut Lines<BufReader<Stdin>>,
    wants_control: bool,
) -> Result<(bool, bool), String> {
    let question = if wants_control {
        "Allow [v]iewing, allow viewing and [t]yping, or [d]eny? "
    } else {
        "Allow [v]iewing, or [d]eny? "
    };
    loop {
        match ask(typed, question).await?.as_str() {
            "v" | "view" | "viewing" => return Ok((true, false)),
            "t" | "type" | "typing" if wants_control => return Ok((true, true)),
            "d" | "deny" | "n" | "no" => return Ok((false, false)),
            _ => {}
        }
    }
}

/// All projects, or the ones chosen by number. Asked only when there is more than one.
async fn which_projects(
    typed: &mut Lines<BufReader<Stdin>>,
    projects: &[Project],
) -> Result<Option<Vec<String>>, String> {
    if projects.len() < 2 {
        return Ok(None);
    }
    for (n, project) in projects.iter().enumerate() {
        println!("  {}. {}", n + 1, project.name);
    }
    loop {
        let answer = ask(
            typed,
            "Which projects? Enter for all of them, or their numbers: ",
        )
        .await?;
        if answer.is_empty() || answer == "all" {
            return Ok(None);
        }
        let chosen: Option<Vec<String>> = answer
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|n| !n.is_empty())
            .map(|n| {
                let i: usize = n.parse().ok()?;
                projects.get(i.checked_sub(1)?).map(|p| p.id.clone())
            })
            .collect();
        match chosen {
            Some(ids) if !ids.is_empty() => return Ok(Some(ids)),
            _ => println!("Numbers from the list, please."),
        }
    }
}

/// A QR code drawn in the terminal, two rows to a character, dark on light.
fn qr(link: &str) -> String {
    match qrcode::QrCode::new(link.as_bytes()) {
        Ok(code) => code
            .render::<qrcode::render::unicode::Dense1x2>()
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
        Err(_) => String::new(),
    }
}
