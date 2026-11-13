//! Eco Mode: the opposite of Game Mode. Lowers what the computer spends and puts everything back afterwards.

use std::thread;
use std::time::{Duration, Instant};

use crate::game_mode::{self, Activation, Item, LenovoMode};
use crate::i18n::Texts;
use crate::storage::{self, Config, EcoRestore, ProgramConfig, ScreenRate, Session};
use crate::system::{self, Screen};

pub const DIMMED_BRIGHTNESS: u8 = 40;
const LOWEST_REFRESH_RATE: u32 = 60;
const SAVER_ALWAYS: u32 = 100;
const SAVER_TIMEOUT: Duration = Duration::from_secs(5);

fn percent(value: u8) -> String {
    format!("{value}%")
}

fn programs(config: &Config) -> Vec<ProgramConfig> {
    config.programs.iter().filter(|p| config.eco.close.contains(&p.name)).cloned().collect()
}

fn charger_item(t: &Texts) -> Option<Item> {
    system::on_ac_power().map(|plugged_in| match plugged_in {
        true => Item::neutral(t.charger, t.plugged_in),
        false => Item::ok(t.charger, t.unplugged),
    })
}

fn power_item(t: &Texts, will_adjust: bool) -> Item {
    let name = |g: &windows_sys::core::GUID| t.power_mode_name(system::power_mode(g));
    match system::effective_power_mode() {
        Some(mode) if system::guid_eq(&mode, &system::BEST_EFFICIENCY) => Item::ok(t.power_mode, name(&mode)),
        Some(mode) if will_adjust => Item::neutral(t.power_mode, name(&mode)),
        Some(mode) => Item::warning(t.power_mode, name(&mode), t.could_not_set_efficiency),
        None => Item::neutral(t.power_mode, t.unknown),
    }
}

/// Energy Saver only comes on while on battery, so being off while plugged in is expected.
fn saver_item(t: &Texts) -> Item {
    match (system::energy_saver_on(), system::on_ac_power()) {
        (true, _) => Item::ok(t.energy_saver, t.saver_on),
        (false, Some(true)) => Item::neutral(t.energy_saver, t.wsl_off).with_hint(t.saver_only_on_battery),
        (false, _) => Item::neutral(t.energy_saver, t.wsl_off),
    }
}

/// The lowest rate the screen offers from 60 Hz up, when it is below the current one.
fn lower_rate(screen: &Screen) -> Option<u32> {
    screen.rates.iter().copied().find(|&rate| rate >= LOWEST_REFRESH_RATE).filter(|&rate| rate < screen.hz)
}

fn screen_items(t: &Texts) -> Vec<Item> {
    system::screens()
        .iter()
        .filter(|s| s.internal)
        .map(|s| {
            let value = t.screen_mode(s.width, s.height, s.hz, false);
            match lower_rate(s) {
                Some(rate) => Item::neutral(t.laptop_screen, value).with_hint(t.can_lower_to(rate)),
                None => Item::ok(t.laptop_screen, value),
            }
        })
        .collect()
}

fn brightness_item(t: &Texts) -> Option<Item> {
    system::brightness().map(|current| match current <= DIMMED_BRIGHTNESS {
        true => Item::ok(t.brightness, percent(current)),
        false => Item::neutral(t.brightness, percent(current)),
    })
}

fn lenovo_item(t: &Texts) -> Option<Item> {
    game_mode::lenovo_mode().map(|mode| match mode {
        LenovoMode::BatterySaver => Item::ok(t.lenovo_mode, mode.label(t)),
        LenovoMode::Unknown => Item::neutral(t.lenovo_mode, mode.label(t)).with_hint(t.lenovo_check),
        LenovoMode::Adaptive | LenovoMode::Performance => Item::neutral(t.lenovo_mode, mode.label(t)).with_hint(t.lenovo_select_battery),
    })
}

fn open_programs_item(config: &Config, t: &Texts) -> Item {
    let all = system::processes();
    let exclude = system::self_and_ancestors(&all);
    let open: Vec<String> = programs(config)
        .into_iter()
        .filter(|p| !system::processes_named(&all, &p.processes, &exclude).is_empty())
        .map(|p| p.name)
        .collect();
    match open.is_empty() {
        true => Item::ok(t.heavy_programs, t.none_open),
        false => Item::neutral(t.heavy_programs, open.join(", ")),
    }
}

/// Current state of the computer, before Eco Mode is activated.
pub fn overview(config: &Config, t: &Texts) -> Vec<Item> {
    let mut items = Vec::new();
    items.extend(charger_item(t));
    items.push(power_item(t, config.eco.power_efficiency));
    items.push(saver_item(t));
    items.extend(screen_items(t));
    items.extend(brightness_item(t));
    items.push(game_mode::wsl_item(t));
    items.push(open_programs_item(config, t));
    items.extend(lenovo_item(t));
    items
}

/// Makes Windows turn Energy Saver on at any battery level; returns the level to put back.
fn turn_on_saver(t: &Texts, items: &mut Vec<Item>) -> Option<u32> {
    let previous = system::saver_threshold();
    let changed = match previous {
        Some(SAVER_ALWAYS) => false,
        Some(_) => system::set_saver_threshold(SAVER_ALWAYS),
        None => false,
    };
    let deadline = Instant::now() + SAVER_TIMEOUT;
    while changed && system::on_ac_power() == Some(false) && !system::energy_saver_on() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
    }
    items.push(match (previous, changed) {
        (Some(SAVER_ALWAYS), _) | (Some(_), true) => saver_item(t),
        (Some(_), false) | (None, _) => Item::warning(t.energy_saver, t.could_not_change, t.power_settings_hint),
    });
    previous.filter(|_| changed)
}

/// Lowers the laptop's own screens to their lowest rate from 60 Hz up; returns the rates to put back.
fn lower_refresh_rates(t: &Texts, items: &mut Vec<Item>) -> Vec<ScreenRate> {
    system::screens()
        .iter()
        .filter(|s| s.internal)
        .filter_map(|s| {
            let unchanged = t.screen_mode(s.width, s.height, s.hz, false);
            let lowered = lower_rate(s).map(|rate| (rate, system::set_refresh_rate(&s.device, rate)));
            items.push(match lowered {
                Some((rate, true)) => Item::ok(t.laptop_screen, t.screen_mode(s.width, s.height, rate, false)),
                Some((_, false)) => Item::warning(t.laptop_screen, unchanged, t.could_not_change),
                None => Item::ok(t.laptop_screen, unchanged),
            });
            lowered.filter(|(_, changed)| *changed).map(|_| ScreenRate { device: s.device.clone(), hz: s.hz })
        })
        .collect()
}

/// Dims the built-in screen when it is brighter than the Eco Mode level; returns the brightness to put back.
fn dim_screen(current: u8, t: &Texts, items: &mut Vec<Item>) -> Option<u8> {
    let dimmed = current > DIMMED_BRIGHTNESS && system::set_brightness(DIMMED_BRIGHTNESS);
    items.push(match (current <= DIMMED_BRIGHTNESS, dimmed) {
        (true, _) => Item::ok(t.brightness, percent(current)),
        (false, true) => Item::ok(t.brightness, percent(DIMMED_BRIGHTNESS)),
        (false, false) => Item::warning(t.brightness, percent(current), t.could_not_change),
    });
    dimmed.then_some(current)
}

/// Closes the chosen programs, shuts down WSL, lowers power, screen rate and brightness, and saves the session.
pub fn activate(config: &Config, t: &Texts, progress: &dyn Fn(&str)) -> Activation {
    let eco = &config.eco;
    let mut items = Vec::new();
    let previous_power = system::chosen_power_mode();
    let brightness_before = eco.dim_screen.then(system::brightness).flatten();

    progress(t.step_closing);
    let closed = game_mode::close_programs(&programs(config));
    items.extend(closed.iter().map(|c| c.item(t)));

    if eco.shutdown_wsl {
        progress(t.step_wsl);
        game_mode::shutdown_wsl_if_idle(t, &mut items);
    }

    progress(t.step_power);
    let power_changed = eco.power_efficiency
        && previous_power.is_some_and(|p| !system::guid_eq(&p, &system::BEST_EFFICIENCY))
        && system::set_power_mode(system::BEST_EFFICIENCY);
    items.extend(charger_item(t));
    items.push(power_item(t, false));
    let saver_threshold = match eco.energy_saver {
        true => turn_on_saver(t, &mut items),
        false => None,
    };

    progress(t.step_screen);
    let refresh_rates = match eco.lower_refresh_rate {
        true => lower_refresh_rates(t, &mut items),
        false => Vec::new(),
    };
    let brightness = brightness_before.and_then(|current| dim_screen(current, t, &mut items));
    items.extend(lenovo_item(t).filter(|i| i.hint.is_some()));

    let session = Session {
        started_at: storage::now_epoch(),
        previous_power_mode: previous_power.filter(|_| power_changed).map(|g| system::guid_to_string(&g)),
        closed_programs: closed.iter().filter(|c| c.closed() && c.had_window).map(|c| c.record.clone()).collect(),
        game: None,
        game_folder: None,
        eco: Some(EcoRestore {
            refresh_rates,
            brightness,
            saver_threshold,
            end_when_plugged_in: eco.end_when_plugged_in && system::on_ac_power().is_some(),
        }),
    };
    let _ = storage::save_session(&session);
    Activation { items, session }
}

/// Puts back the Energy Saver level, the refresh rates and the brightness that Eco Mode changed.
pub fn restore(eco: &EcoRestore, t: &Texts) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(level) = eco.saver_threshold {
        items.push(match system::set_saver_threshold(level) {
            true => Item::ok(t.energy_saver, t.restored),
            false => Item::warning(t.energy_saver, t.could_not_restore, t.power_settings_hint),
        });
    }
    for rate in &eco.refresh_rates {
        items.push(match system::set_refresh_rate(&rate.device, rate.hz) {
            true => Item::ok(t.laptop_screen, t.power_back_to(&format!("{} Hz", rate.hz))),
            false => Item::warning(t.laptop_screen, t.could_not_restore, t.supports_hz(rate.hz)),
        });
    }
    if let Some(previous) = eco.brightness {
        items.push(match system::set_brightness(previous) {
            true => Item::ok(t.brightness, t.power_back_to(&percent(previous))),
            false => Item::warning(t.brightness, t.could_not_restore, t.brightness_hint),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::PT_BR;

    fn screen(hz: u32, rates: &[u32]) -> Screen {
        Screen { name: String::new(), device: String::new(), internal: true, primary: true, width: 1920, height: 1200, hz, max_hz: hz, rates: rates.to_vec() }
    }

    #[test]
    fn lowers_only_to_a_rate_from_60_hz_up_and_below_the_current_one() {
        assert_eq!(lower_rate(&screen(144, &[48, 60, 120, 144])), Some(60));
        assert_eq!(lower_rate(&screen(120, &[90, 120])), Some(90));
        assert_eq!(lower_rate(&screen(60, &[48, 60])), None);
        assert_eq!(lower_rate(&screen(60, &[60, 120])), None);
        assert_eq!(lower_rate(&screen(48, &[48])), None);
    }

    #[test]
    #[ignore]
    fn print_overview() {
        for item in overview(&Config::default(), &PT_BR) {
            println!("{:<30} {:<40} {:?} {}", item.label, item.value, item.status, item.hint.unwrap_or_default());
        }
        println!("saver threshold on battery: {:?}", system::saver_threshold());
    }
}
