//! Chat-log monitoring and the activity overlays on Windows: runs eve_maj_core's ChatlogWorker (system names,
//! notifications with their sounds and speech, character IDs), the combat/mining/bounty trackers it feeds from game
//! logs, and per-client CPU/RAM/VRAM sampling, pushing results into the painter's thumbnails. The Zig build's
//! chatlog.zig plus main.zig's updateThrottledTracker/push*Update, and activity/resources.zig.
//!
//! The worker thread never touches the UI globals: it wakes the timer window with WM_CHATLOG_EVENTS and the events
//! are applied there. State lives in its own global (not the painter), so it survives a profile reload and only
//! restarts when the chat-log or tracker settings actually change.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eve_maj_core::activity_tracker::{parse_gpu_instance_pid, BountyTracker, CombatTracker, MiningTracker};
use eve_maj_core::chatlog::{ActivityTrackers, ChatlogEvent, ChatlogMonitor, ChatlogWorker, WorkerCommand};
use eve_maj_core::config::{ChatlogConfig, Config, GlobalSettings};
use eve_maj_core::log::Scope;
use eve_maj_core::types::NotificationType;
use eve_maj_win::time::Ticks;
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, FILETIME, HWND};
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE, PDH_MORE_DATA,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{GetActiveProcessorCount, GetProcessTimes, OpenProcess, ALL_PROCESSOR_GROUPS, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::globals::{self, Global};
use crate::scout::is_generic_character_name;

const SLOG: Scope = Scope::new("chatlog");
const RLOG: Scope = Scope::new("activity");

/// Posted by the worker thread to the timer window when events are waiting.
pub const WM_CHATLOG_EVENTS: u32 = WM_APP + 40;

/// One wake message in flight at a time; cleared when the timer window drains the queue.
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

static MONITOR: Global<Monitor> = Global::new();

/// The settings that decide which trackers exist; a change recreates them (and restarts the worker that feeds them).
#[derive(Debug, Clone, PartialEq)]
struct TrackerKey {
    combat: Option<(u32, String)>,
    mining: Option<u32>,
    bounty: Option<u32>,
}

impl TrackerKey {
    fn from(config: &Config) -> Self {
        TrackerKey {
            combat: config.combat.enabled.then(|| (config.combat.window_seconds, config.combat.damage_alert_excluded_weapons.clone())),
            mining: config.mining.enabled.then_some(config.mining.window_seconds),
            bounty: config.bounty.enabled.then_some(config.bounty.window_seconds),
        }
    }
}

#[derive(Default)]
struct Monitor {
    worker: Option<ChatlogWorker>,
    /// Characters the worker has been told about.
    characters: Vec<String>,
    /// What the running worker was started with; a change to either restarts it.
    config: Option<(ChatlogConfig, TrackerKey)>,
    /// Log directories that were missing, so the warning is logged once per directory pair.
    warned_missing: Option<(String, String)>,
    combat: Option<Arc<CombatTracker>>,
    mining: Option<Arc<MiningTracker>>,
    bounty: Option<Arc<BountyTracker>>,
    last_combat: Ticks,
    last_mining: Ticks,
    last_bounty: Ticks,
    last_resources: Ticks,
    resources: Option<Resources>,
}

/// Fresh trackers for `config`, and the handles the chat-log worker feeds.
fn new_trackers(config: &Config, global: &GlobalSettings) -> (Option<Arc<CombatTracker>>, Option<Arc<MiningTracker>>, Option<Arc<BountyTracker>>, ActivityTrackers) {
    let combat = config.combat.enabled.then(|| Arc::new(CombatTracker::new(config.combat.window_seconds)));
    let mining = config.mining.enabled.then(|| Arc::new(MiningTracker::new(config.mining.window_seconds)));
    let bounty = config.bounty.enabled.then(|| Arc::new(BountyTracker::new(config.bounty.window_seconds)));
    // Prices can be overridden in global settings; a private copy answers the worker's lookups.
    let mut ore_settings = GlobalSettings::default();
    ore_settings.ore_table = global.ore_table.clone();
    let feeds = ActivityTrackers {
        combat: combat.clone(),
        mining: mining.clone(),
        bounty: bounty.clone(),
        damage_alert_excluded_weapons: config.combat.damage_alert_excluded_weapons.clone(),
        ore: mining.is_some().then(|| {
            Box::new(move |name: &str| ore_settings.ore_volume(name).map(|v| (v, ore_settings.ore_price(name).unwrap_or(0.0)))) as Box<dyn Fn(&str) -> Option<(f64, f64)> + Send>
        }),
    };
    (combat, mining, bounty, feeds)
}

/// Starts, restarts or stops monitoring to match the live profile; call after startup, a profile reload and each scan.
pub fn sync() {
    let monitor = match MONITOR.get() {
        Some(m) => m,
        None => {
            MONITOR.set(Monitor::default());
            MONITOR.get().expect("set above")
        }
    };
    let config = globals::config();
    let wanted = (config.chatlog.clone(), TrackerKey::from(config));
    let enabled = config.chatlog.enabled;

    if monitor.worker.is_some() && (!enabled || monitor.config.as_ref() != Some(&wanted)) {
        SLOG.info(format_args!("{}", if enabled { "Chat-log or tracker settings changed, restarting monitoring" } else { "Chat-log monitoring turned off" }));
        stop(monitor);
    }

    // Resource sampling doesn't need the chat logs.
    if config.resources.enabled && monitor.resources.is_none() {
        monitor.resources = Some(Resources::new());
    } else if !config.resources.enabled && monitor.resources.is_some() {
        monitor.resources = None;
    }

    if !enabled || monitor.worker.is_some() {
        sync_characters(monitor);
        return;
    }
    let (chat, game) = (PathBuf::from(&config.chatlog.chatlog_dir), PathBuf::from(&config.chatlog.gamelog_dir));
    if !chat.is_dir() || !game.is_dir() {
        let key = (config.chatlog.chatlog_dir.clone(), config.chatlog.gamelog_dir.clone());
        if monitor.warned_missing.as_ref() != Some(&key) {
            SLOG.warn(format_args!(
                "Chat-log monitoring is on but EVE's log folders weren't found (chat logs: {} [{}], game logs: {} [{}]); set them on the settings window's Chatlog tab",
                chat.display(),
                if chat.is_dir() { "found" } else { "missing" },
                game.display(),
                if game.is_dir() { "found" } else { "missing" }
            ));
            monitor.warned_missing = Some(key);
        }
        return;
    }
    let Some(timer) = globals::timer_hwnd() else { return };
    let gs = globals::GLOBAL_SETTINGS.get().expect("global settings are loaded first");

    let mut chatlog_monitor = ChatlogMonitor::new(&chat, &game, config.chatlog.idle_poll_threshold, config.chatlog.max_poll_multiplier, gs.character_id_map_snapshot());
    let (combat, mining, bounty, feeds) = new_trackers(config, gs);
    chatlog_monitor.trackers = feeds;
    // HWND isn't Send; the worker only ever posts to it.
    let timer_addr = timer as usize;
    let wake = move || {
        if !WAKE_PENDING.swap(true, Ordering::AcqRel) {
            unsafe { PostMessageW(timer_addr as HWND, WM_CHATLOG_EVENTS, 0, 0) };
        }
    };
    match ChatlogWorker::spawn(chatlog_monitor, Duration::from_millis(config.chatlog.poll_interval_ms.max(50) as u64), wake) {
        Ok(worker) => {
            SLOG.info(format_args!(
                "Chat-log monitoring started: {} and {} (poll {} ms; trackers: combat {}, mining {}, bounty {})",
                chat.display(),
                game.display(),
                config.chatlog.poll_interval_ms,
                on_off(combat.is_some()),
                on_off(mining.is_some()),
                on_off(bounty.is_some())
            ));
            monitor.worker = Some(worker);
            monitor.config = Some(wanted);
            monitor.warned_missing = None;
            (monitor.combat, monitor.mining, monitor.bounty) = (combat, mining, bounty);
            monitor.characters.clear();
            sync_characters(monitor);
        }
        Err(err) => SLOG.err(format_args!("Couldn't start the chat-log worker: {err}")),
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn stop(monitor: &mut Monitor) {
    if let Some(worker) = monitor.worker.take() {
        worker.shutdown();
    }
    monitor.config = None;
    monitor.characters.clear();
    (monitor.combat, monitor.mining, monitor.bounty) = (None, None, None);
    if let Some(painter) = globals::painter() {
        for t in &mut painter.thumbnails {
            (t.has_dps_data, t.has_mining_data, t.has_bounty_data) = (false, false, false);
            t.needs_render = true;
        }
    }
}

/// Stops the worker thread; call before the timer window goes away.
pub fn shutdown() {
    if let Some(mut monitor) = MONITOR.take() {
        if let Some(worker) = monitor.worker.take() {
            worker.shutdown();
        }
    }
}

/// Pushes every reading again on the next tick (after a profile reload rebuilt the thumbnails).
pub fn force_refresh() {
    if let Some(m) = MONITOR.get() {
        (m.last_combat, m.last_mining, m.last_bounty, m.last_resources) = (Ticks::default(), Ticks::default(), Ticks::default(), Ticks::default());
    }
}

/// Tells the worker about logins and logouts (any change in the set of logged-in character names).
fn sync_characters(monitor: &mut Monitor) {
    let Some(worker) = monitor.worker.as_ref() else { return };
    let current: Vec<String> = globals::scout()
        .map(|s| s.windows.iter().map(|w| w.character_name.clone()).filter(|n| !is_generic_character_name(n)).collect())
        .unwrap_or_default();
    for gone in monitor.characters.iter().filter(|n| !current.contains(n)) {
        SLOG.debug(format_args!("Stopped watching logs for {gone}"));
        worker.send(WorkerCommand::RemoveCharacter(gone.clone()));
        for t in monitor.combat.iter() {
            t.remove_character(gone);
        }
        for t in monitor.mining.iter() {
            t.remove_character(gone);
        }
        for t in monitor.bounty.iter() {
            t.remove_character(gone);
        }
    }
    for new in current.iter().filter(|n| !monitor.characters.contains(n)) {
        SLOG.debug(format_args!("Watching logs for {new}"));
        worker.send(WorkerCommand::AddCharacter(new.clone()));
    }
    monitor.characters = current;
}

/// WM_CHATLOG_EVENTS: applies what the worker found.
pub fn on_events() {
    WAKE_PENDING.store(false, Ordering::Release);
    let Some(monitor) = MONITOR.get() else { return };
    let events = monitor.worker.as_ref().map(|w| w.try_events()).unwrap_or_default();
    for event in events {
        match event {
            ChatlogEvent::System { character, system, event_ts, is_jump } => match hwnd_for(&character) {
                Some(hwnd) => {
                    SLOG.debug(format_args!("{character}: system {system}{}", if is_jump { " (jump)" } else { "" }));
                    if let Some(painter) = globals::painter() {
                        painter.update_system_name_by_hwnd(hwnd, &system, event_ts, is_jump);
                    }
                }
                None => SLOG.debug(format_args!("No client for {character}, skipping system update")),
            },
            ChatlogEvent::Notification { character, text, ntype } => match hwnd_for(&character) {
                Some(hwnd) => {
                    SLOG.debug(format_args!("{character}: [{}] {text}", ntype.as_str()));
                    if let Some(painter) = globals::painter() {
                        painter.show_notification(hwnd, &text, ntype);
                    }
                }
                None => SLOG.debug(format_args!("No client for {character}, skipping notification")),
            },
            ChatlogEvent::CharacterId { character, id } => {
                if let Some(gs) = globals::GLOBAL_SETTINGS.get() {
                    if let Err(err) = gs.update_character_id(&character, &id) {
                        SLOG.warn(format_args!("Couldn't cache character ID for {character}: {err}"));
                    }
                }
            }
        }
    }
    if let Some(painter) = globals::painter() {
        painter.render_dirty_thumbnails(None);
    }
}

fn hwnd_for(character: &str) -> Option<HWND> {
    globals::painter()?.thumbnails.iter().find(|t| t.character_name == character).map(|t| t.source_hwnd)
}

fn due(last: &mut Ticks, now: Ticks, interval_ms: u32) -> bool {
    if !last.is_zero() && now.elapsed_since(*last) < interval_ms as u64 {
        return false;
    }
    *last = now;
    true
}

fn now_epoch_ms() -> i64 {
    eve_maj_core::render::now_ms() as i64
}

/// Per tick: pushes tracker readings and resource stats into thumbnails (each on its own interval) and raises the
/// trackers' alerts (Taking damage, Laser idle, Mining stopped).
pub fn tick() {
    let Some(monitor) = MONITOR.get() else { return };
    let Some(painter) = globals::painter() else { return };
    let config = globals::config();
    let now = Ticks::now();
    let now_ms = now_epoch_ms();
    let names: Vec<(HWND, String)> = painter.thumbnails.iter().map(|t| (t.source_hwnd, t.character_name.clone())).collect();
    let mut alerts: Vec<(HWND, &'static str, NotificationType)> = Vec::new();

    if let Some(tracker) = monitor.combat.clone() {
        if due(&mut monitor.last_combat, now, config.combat.update_interval_ms) {
            tracker.refresh_all(now_ms);
            for (hwnd, name) in &names {
                let dps = tracker.get_dps(name);
                painter.update_dps_for_character(*hwnd, dps.incoming, dps.outgoing);
                if tracker.check_damage_alert(name, now_ms) {
                    alerts.push((*hwnd, "Taking damage", NotificationType::TakingDamage));
                }
            }
        }
    }

    if let Some(tracker) = monitor.mining.clone() {
        if due(&mut monitor.last_mining, now, config.mining.update_interval_ms) {
            tracker.refresh_all(now_ms);
            let idle_window = config.mining.idle_alert_window_seconds as i64 * 1000;
            let idle_threshold = config.mining.idle_alert_threshold;
            let stopped_window = config.mining.stopped_alert_window_seconds as i64 * 1000;
            for (hwnd, name) in &names {
                painter.update_mining_for_character(*hwnd, tracker.get_rate(name), tracker.get_isk_rate(name));
                if tracker.check_idle_alert(name, now_ms, idle_window, idle_threshold) {
                    alerts.push((*hwnd, "Laser idle", NotificationType::MiningIdle));
                }
                if tracker.check_stopped_alert(name, now_ms, stopped_window) {
                    alerts.push((*hwnd, "Mining stopped", NotificationType::MiningStopped));
                }
            }
        }
    }

    if let Some(tracker) = monitor.bounty.clone() {
        if due(&mut monitor.last_bounty, now, config.bounty.update_interval_ms) {
            tracker.refresh_all(now_ms);
            for (hwnd, name) in &names {
                painter.update_bounty_for_character(*hwnd, tracker.get_isk_rate(name));
            }
        }
    }

    if let Some(resources) = monitor.resources.as_mut() {
        if due(&mut monitor.last_resources, now, config.resources.update_interval_ms) {
            if let Some(scout) = globals::scout() {
                let pids: Vec<(HWND, u32)> = scout.windows.iter().map(|w| (w.hwnd, w.process_id)).collect();
                resources.sample(&pids, now);
                for (hwnd, pid) in pids {
                    if let Some(s) = resources.stats.get(&pid) {
                        painter.update_resource_stats_for_character(hwnd, s.cpu_percent, s.ram_mb, s.vram_mb, s.has_vram);
                    }
                }
            }
        }
    }

    for (hwnd, text, ntype) in alerts {
        painter.show_notification(hwnd, text, ntype);
    }
    painter.process_dirty_dps_overlays();
}

// ------------------------------------------------------------------------------------------------- resources --

/// Instance names look like "pid_1234_luid_0x0_0xABCD_phys_0", one per process per physical adapter.
const VRAM_COUNTER_PATH: &str = "\\GPU Process Memory(*)\\Dedicated Usage";
const BYTES_PER_MB: f32 = 1024.0 * 1024.0;
/// FILETIME counts 100 ns intervals.
const FILETIME_TICKS_PER_MS: u64 = 10_000;

#[derive(Default, Clone, Copy)]
struct Stats {
    cpu_percent: f32,
    ram_mb: f32,
    vram_mb: f32,
    has_vram: bool,
}

struct Resources {
    /// pid -> (CPU ms, when sampled).
    cpu_samples: HashMap<u32, (u64, Ticks)>,
    stats: HashMap<u32, Stats>,
    logical_processors: f64,
    /// PDH query and counter for VRAM; None when the GPU counters aren't available (older Windows, some drivers).
    pdh: Option<(isize, isize)>,
    pdh_failed_logged: bool,
}

impl Resources {
    fn new() -> Resources {
        let logical_processors = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }.max(1) as f64;
        let pdh = unsafe {
            let mut query: isize = 0;
            let status = windows_sys::Win32::System::Performance::PdhOpenQueryW(std::ptr::null(), 0, &mut query);
            if status != 0 {
                RLOG.warn(format_args!("Failed to open a PDH query, so VRAM use won't show: 0x{status:X}"));
                None
            } else {
                let mut counter: isize = 0;
                let path = eve_maj_win::wide(VRAM_COUNTER_PATH);
                // English variant: PdhAddCounter wants OS-localised counter names, which would break on non-English Windows.
                let status = PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter);
                if status != 0 {
                    RLOG.warn(format_args!("Failed to add PDH counter '{VRAM_COUNTER_PATH}', so VRAM use won't show: 0x{status:X}"));
                    PdhCloseQuery(query);
                    None
                } else {
                    Some((query, counter))
                }
            }
        };
        RLOG.info(format_args!("Resource sampling started ({logical_processors} logical processors, VRAM {})", if pdh.is_some() { "available" } else { "unavailable" }));
        Resources { cpu_samples: HashMap::new(), stats: HashMap::new(), logical_processors, pdh, pdh_failed_logged: false }
    }

    fn sample(&mut self, clients: &[(HWND, u32)], now: Ticks) {
        for &(_, pid) in clients {
            if pid != 0 {
                self.sample_cpu_and_ram(pid, now);
            }
        }
        if self.pdh.is_some() {
            if let Err(what) = self.poll_vram(clients) {
                if !self.pdh_failed_logged {
                    RLOG.warn(format_args!("Failed to poll VRAM use: {what}"));
                    self.pdh_failed_logged = true;
                }
            }
        }
        let live: Vec<u32> = clients.iter().map(|&(_, p)| p).collect();
        self.stats.retain(|pid, _| live.contains(pid));
        self.cpu_samples.retain(|pid, _| live.contains(pid));
    }

    fn sample_cpu_and_ram(&mut self, pid: u32, now: Ticks) {
        unsafe {
            // Not logged: this runs every sample, and fails only for a client that has just closed.
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if handle.is_null() {
                return;
            }
            let entry = self.stats.entry(pid).or_default();
            let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
            let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
            if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0 {
                let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
                let cpu_ms = (ft(kernel) + ft(user)) / FILETIME_TICKS_PER_MS;
                if let Some(&(prev_ms, prev_at)) = self.cpu_samples.get(&pid) {
                    let wall = now.elapsed_since(prev_at);
                    if wall > 0 && cpu_ms >= prev_ms {
                        // Share of the whole machine: process CPU time / wall time / logical processors.
                        entry.cpu_percent = ((cpu_ms - prev_ms) as f64 / wall as f64 / self.logical_processors * 100.0).clamp(0.0, 100.0) as f32;
                    }
                }
                self.cpu_samples.insert(pid, (cpu_ms, now));
            }
            let mut mem: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(handle, &mut mem, mem.cb) != 0 {
                entry.ram_mb = mem.WorkingSetSize as f32 / BYTES_PER_MB;
            }
            CloseHandle(handle);
        }
    }

    /// One wildcard query covers every process; each instance is matched to a client by its pid.
    fn poll_vram(&mut self, clients: &[(HWND, u32)]) -> Result<(), String> {
        let Some((query, counter)) = self.pdh else { return Ok(()) };
        for &(_, pid) in clients {
            if let Some(s) = self.stats.get_mut(&pid) {
                (s.vram_mb, s.has_vram) = (0.0, false);
            }
        }
        unsafe {
            let status = PdhCollectQueryData(query);
            if status != 0 {
                return Err(format!("PdhCollectQueryData 0x{status:X}"));
            }
            let (mut size, mut count) = (0u32, 0u32);
            let status = PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, std::ptr::null_mut());
            if status != PDH_MORE_DATA || size == 0 {
                return Ok(());
            }
            // u64-aligned backing store for the PDH_FMT_COUNTERVALUE_ITEM_W array plus the strings PDH appends after it.
            let mut buf: Vec<u64> = vec![0; (size as usize).div_ceil(8)];
            let status = PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W);
            if status != 0 {
                return Err(format!("PdhGetFormattedCounterArrayW 0x{status:X}"));
            }
            let items = std::slice::from_raw_parts(buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W, count as usize);
            for item in items {
                // PDH_CSTATUS_VALID_DATA (0) and PDH_CSTATUS_NEW_DATA (1) both carry a usable value.
                if item.FmtValue.CStatus > 1 || item.szName.is_null() {
                    continue;
                }
                let mut len = 0;
                while *item.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len));
                let Some(pid) = parse_gpu_instance_pid(&name) else { continue };
                if let Some(s) = self.stats.get_mut(&pid) {
                    s.vram_mb += item.FmtValue.Anonymous.largeValue as f32 / BYTES_PER_MB;
                    s.has_vram = true;
                }
            }
        }
        Ok(())
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        if let Some((query, _)) = self.pdh.take() {
            unsafe { PdhCloseQuery(query) };
        }
    }
}
