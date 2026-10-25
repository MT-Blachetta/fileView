//! Scripted input for automated UI checks (`PLANAR_SCRIPT=file`).
//!
//! One command per line, executed frame by frame:
//! `wait N`, `move X Y`, `click X Y`, `dclick X Y`, `rclick X Y`, `mclick X Y`, `tap MOD`,
//! `drag X1 Y1 X2 Y2 [ctrl]`, `scroll X Y DY`, `key NAME [ctrl] [shift] [alt]`, `type TEXT`,
//! `copy`, `cut`, `paste`, `shot FILE.ppm`, `print`, `quit`. Lines starting with `#` are ignored.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, pos2};
use std::collections::VecDeque;

enum Step {
    Wait(u32),
    Events(Vec<Event>, Modifiers),
    Shot(String),
    Print,
    Quit,
}

pub struct Script {
    steps: VecDeque<Step>,
    pub pending_shot: Option<String>,
    pub want_print: bool,
}

fn pos(a: &[&str], i: usize) -> Pos2 {
    pos2(a.get(i).and_then(|v| v.parse().ok()).unwrap_or(0.0), a.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.0))
}

fn mods(a: &[&str]) -> Modifiers {
    let mut m = Modifiers::NONE;
    for w in a {
        match *w {
            "ctrl" => {
                m.ctrl = true;
                m.command = true;
            }
            "shift" => m.shift = true,
            "alt" => m.alt = true,
            _ => {}
        }
    }
    m
}

fn button(p: Pos2, b: PointerButton, pressed: bool, m: Modifiers) -> Event {
    Event::PointerButton { pos: p, button: b, pressed, modifiers: m }
}

impl Script {
    pub fn load(path: &str) -> Option<Script> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut steps = VecDeque::new();
        for line in text.lines() {
            let a: Vec<&str> = line.split_whitespace().collect();
            let Some(cmd) = a.first() else { continue };
            let m = mods(&a);
            match *cmd {
                c if c.starts_with('#') => {}
                "wait" => steps.push_back(Step::Wait(a.get(1).and_then(|v| v.parse().ok()).unwrap_or(1))),
                "move" => steps.push_back(Step::Events(vec![Event::PointerMoved(pos(&a, 1))], m)),
                "tap" => {
                    // Press and release a modifier on its own, e.g. `tap alt`.
                    steps.push_back(Step::Events(vec![], m));
                    steps.push_back(Step::Wait(1));
                    steps.push_back(Step::Events(vec![], Modifiers::NONE));
                    steps.push_back(Step::Wait(2));
                }
                "click" | "dclick" | "rclick" | "mclick" => {
                    let p = pos(&a, 1);
                    let b = match *cmd {
                        "rclick" => PointerButton::Secondary,
                        "mclick" => PointerButton::Middle,
                        _ => PointerButton::Primary,
                    };
                    let n = if *cmd == "dclick" { 2 } else { 1 };
                    steps.push_back(Step::Events(vec![Event::PointerMoved(p)], m));
                    steps.push_back(Step::Wait(1));
                    for _ in 0..n {
                        steps.push_back(Step::Events(vec![button(p, b, true, m)], m));
                        steps.push_back(Step::Events(vec![button(p, b, false, m)], m));
                    }
                    steps.push_back(Step::Wait(2));
                }
                "drag" => {
                    let p1 = pos(&a, 1);
                    let p2 = pos(&a, 3);
                    steps.push_back(Step::Events(vec![Event::PointerMoved(p1)], m));
                    steps.push_back(Step::Wait(1));
                    steps.push_back(Step::Events(vec![button(p1, PointerButton::Primary, true, m)], m));
                    for k in 1..=12 {
                        let t = k as f32 / 12.0;
                        steps.push_back(Step::Events(vec![Event::PointerMoved(p1 + (p2 - p1) * t)], m));
                    }
                    steps.push_back(Step::Wait(2));
                    steps.push_back(Step::Events(vec![button(p2, PointerButton::Primary, false, m)], m));
                    steps.push_back(Step::Wait(3));
                }
                "key" => {
                    if let Some(k) = a.get(1).and_then(|n| Key::from_name(n)) {
                        let ev = |pressed| Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: m };
                        steps.push_back(Step::Events(vec![ev(true)], m));
                        steps.push_back(Step::Events(vec![ev(false)], m));
                        steps.push_back(Step::Wait(2));
                    }
                }
                "type" => {
                    let t = line.trim_start().strip_prefix("type").unwrap_or("").trim_start().to_string();
                    steps.push_back(Step::Events(vec![Event::Text(t)], m));
                    steps.push_back(Step::Wait(1));
                }
                "scroll" => {
                    let p = pos(&a, 1);
                    let dy: f32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    steps.push_back(Step::Events(vec![Event::PointerMoved(p)], m));
                    for _ in 0..5 {
                        steps.push_back(Step::Events(
                            vec![Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, dy / 5.0),
                                phase: egui::TouchPhase::Move,
                                source: egui::MouseWheelSource::Wheel,
                                modifiers: m,
                            }],
                            m,
                        ));
                    }
                    steps.push_back(Step::Wait(20));
                }
                "copy" => steps.push_back(Step::Events(vec![Event::Copy], m)),
                "cut" => steps.push_back(Step::Events(vec![Event::Cut], m)),
                "paste" => steps.push_back(Step::Events(vec![Event::Paste(String::new())], m)),
                "shot" => steps.push_back(Step::Shot(a.get(1).unwrap_or(&"shot.ppm").to_string())),
                "print" => steps.push_back(Step::Print),
                "quit" => steps.push_back(Step::Quit),
                _ => eprintln!("script: unknown command {line}"),
            }
        }
        Some(Script { steps, pending_shot: None, want_print: false })
    }

    /// Feeds the next step into this frame's input. Returns false when the script asks to quit.
    pub fn feed(&mut self, raw: &mut RawInput) -> bool {
        loop {
            match self.steps.front_mut() {
                None => return true,
                Some(Step::Wait(n)) => {
                    if *n == 0 {
                        self.steps.pop_front();
                        continue;
                    }
                    *n -= 1;
                    return true;
                }
                Some(Step::Events(..)) => {
                    let Some(Step::Events(ev, m)) = self.steps.pop_front() else { unreachable!() };
                    raw.events.push(Event::ModifiersChanged(m));
                    raw.events.extend(ev);
                    return true;
                }
                Some(Step::Shot(_)) => {
                    let Some(Step::Shot(p)) = self.steps.pop_front() else { unreachable!() };
                    self.pending_shot = Some(p);
                    return true;
                }
                Some(Step::Print) => {
                    self.steps.pop_front();
                    self.want_print = true;
                    return true;
                }
                Some(Step::Quit) => return false,
            }
        }
    }
}
