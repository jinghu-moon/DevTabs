//! A physical press keeps its original owner until release, including across overlays.
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalKey {
    pub vk: u16,
    pub scan: u16,
    pub enhanced: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawKey {
    pub physical: PhysicalKey,
    pub unicode: u16,
    pub down: bool,
    pub control: u32,
    pub repeat: u16,
    pub bytes: Vec<u8>,
}
impl RawKey {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let text = std::str::from_utf8(bytes).ok()?;
        let first = text.strip_prefix("\x1b[")?.split('_').next()?;
        let values = first
            .split(';')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if values.len() != 6 {
            return None;
        }
        Some(Self {
            physical: PhysicalKey {
                vk: values[0] as u16,
                scan: values[1] as u16,
                enhanced: values[4] & 0x100 != 0,
            },
            unicode: values[2] as u16,
            down: values[3] != 0,
            control: values[4],
            repeat: values[5] as u16,
            bytes: bytes.into(),
        })
    }
    pub fn release(&self, control: u32) -> Vec<u8> {
        format!(
            "\x1b[{};{};{};0;{};1_",
            self.physical.vk, self.physical.scan, self.unicode, control
        )
        .into_bytes()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub session: String,
    pub generation: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireMode {
    Win32,
    Vt { application_cursor: bool },
}
#[derive(Clone, Debug)]
pub enum Owner {
    Ui { context: u64, repeat: bool },
    Terminal { target: Target, wire: WireMode },
    SequencePending,
    SuppressUntilUp,
}
#[derive(Clone, Debug)]
struct Press {
    owner: Owner,
    first: RawKey,
    delivered: bool,
}
#[derive(Default)]
pub struct Ledger {
    held: BTreeMap<PhysicalKey, Press>,
    pub orphan_ups: u64,
    pub failed_balances: u64,
    pub epoch: u64,
}
#[derive(Clone, Debug)]
pub enum Existing {
    New,
    UiRepeat,
    Consume,
    Forward { target: Target, wire: WireMode },
}
impl Ledger {
    /// Called before modal routing, even for standalone modifier records.
    pub fn existing(&mut self, raw: &RawKey, context: u64) -> Existing {
        if raw.down {
            match self.held.get(&raw.physical) {
                None => Existing::New,
                Some(press) => match &press.owner {
                    Owner::Terminal { target, wire } => Existing::Forward {
                        target: target.clone(),
                        wire: *wire,
                    },
                    Owner::Ui {
                        context: owner,
                        repeat: true,
                    } if *owner == context => Existing::UiRepeat,
                    _ => Existing::Consume,
                },
            }
        } else {
            match self.held.remove(&raw.physical) {
                Some(press) => match press.owner {
                    Owner::Terminal { target, wire } if press.delivered => {
                        Existing::Forward { target, wire }
                    }
                    _ => Existing::Consume,
                },
                None => {
                    self.orphan_ups += 1;
                    Existing::Consume
                }
            }
        }
    }
    pub fn own(&mut self, raw: RawKey, owner: Owner, delivered: bool) {
        if raw.down {
            self.held.entry(raw.physical).or_insert(Press {
                owner,
                first: raw,
                delivered,
            });
        }
    }
    pub fn context_changed(&mut self) {
        for press in self.held.values_mut() {
            if !matches!(press.owner, Owner::Terminal { .. }) {
                press.owner = Owner::SuppressUntilUp;
            }
        }
    }
    /// Balance actual Win32 downs on the original connection; ordinary keys precede modifiers.
    pub fn blur(&mut self, mut send: impl FnMut(&Target, Vec<u8>) -> bool) {
        let mut keys = self.held.keys().copied().collect::<Vec<_>>();
        keys.sort_by_key(|key| matches!(key.vk,16..=18|160..=165));
        let mut controls = BTreeMap::<Target, u32>::new();
        for key in keys {
            let press = self.held.get_mut(&key).unwrap();
            if let Owner::Terminal {
                target,
                wire: WireMode::Win32,
            } = &press.owner
                && press.delivered
            {
                let control = controls
                    .entry(target.clone())
                    .or_insert(press.first.control);
                *control &= !match key.vk {
                    16 | 160 | 161 => 0x10,
                    17 | 162 | 163 => 0xc,
                    18 | 164 | 165 => 0x3,
                    _ => 0,
                };
                if !send(target, press.first.release(*control)) {
                    self.failed_balances += 1;
                }
                press.delivered = false;
            }
            press.owner = Owner::SuppressUntilUp;
        }
    }
    pub fn focus(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }
    pub fn disconnect(&mut self, target: &Target) {
        for press in self.held.values_mut() {
            if matches!(&press.owner,Owner::Terminal{target:t,..} if t==target) {
                if press.delivered {
                    self.failed_balances += 1;
                }
                press.delivered = false;
                press.owner = Owner::SuppressUntilUp;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn raw(vk: u16, down: bool, control: u32) -> RawKey {
        RawKey::parse(format!("\x1b[{vk};23;0;{};{control};1_", u8::from(down)).as_bytes()).unwrap()
    }
    fn target() -> Target {
        Target {
            session: "original".into(),
            generation: "connection-1".into(),
        }
    }
    #[test]
    fn release_and_repeat_keep_original_target_after_modal_or_modifier_change() {
        let mut ledger = Ledger::default();
        let down = raw(114, true, 8);
        let t = target();
        assert!(matches!(ledger.existing(&down, 1), Existing::New));
        ledger.own(
            down.clone(),
            Owner::Terminal {
                target: t.clone(),
                wire: WireMode::Win32,
            },
            true,
        );
        ledger.context_changed();
        assert!(matches!(ledger.existing(&down,2),Existing::Forward{target,..} if target==t));
        assert!(
            matches!(ledger.existing(&raw(114,false,0),2),Existing::Forward{target,..} if target==t)
        );
        assert!(matches!(
            ledger.existing(&raw(114, false, 0), 2),
            Existing::Consume
        ));
        assert_eq!(ledger.orphan_ups, 1);
    }
    #[test]
    fn blur_balances_only_delivered_win32_downs_once_and_suppresses_real_ups() {
        let mut ledger = Ledger::default();
        let t = target();
        for vk in [17, 65] {
            ledger.own(
                raw(vk, true, 8),
                Owner::Terminal {
                    target: t.clone(),
                    wire: WireMode::Win32,
                },
                true,
            );
        }
        ledger.own(
            raw(114, true, 8),
            Owner::Ui {
                context: 1,
                repeat: false,
            },
            false,
        );
        let mut sent = vec![];
        ledger.blur(|target, bytes| {
            sent.push((target.clone(), bytes));
            true
        });
        assert_eq!(sent.len(), 2);
        assert!(sent[0].1.starts_with(b"\x1b[65;"));
        assert!(sent[1].1.ends_with(b";0;0;1_"));
        ledger.blur(|_, _| panic!("duplicate balance"));
        ledger.focus();
        for vk in [17, 65, 114] {
            assert!(matches!(
                ledger.existing(&raw(vk, false, 0), 2),
                Existing::Consume
            ));
        }
    }
    #[test]
    fn ui_once_suppresses_repeats_and_reconnected_generation_never_inherits_old_up() {
        let mut ledger = Ledger::default();
        let key = raw(114, true, 8);
        ledger.own(
            key.clone(),
            Owner::Ui {
                context: 1,
                repeat: false,
            },
            false,
        );
        assert!(matches!(ledger.existing(&key, 1), Existing::Consume));
        ledger.existing(&raw(114, false, 0), 1);
        ledger.own(
            key,
            Owner::Terminal {
                target: target(),
                wire: WireMode::Vt {
                    application_cursor: false,
                },
            },
            true,
        );
        ledger.disconnect(&target());
        assert!(matches!(
            ledger.existing(&raw(114, false, 0), 2),
            Existing::Consume
        ));
    }
}
