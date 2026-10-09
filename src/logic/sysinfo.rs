//! A snapshot of this computer for bug reports: system, hardware, storage,
//! network. Nothing leaves the machine; identifying fields can be left out.

use serde_json::{Map, Value};
use sysinfo::{Disks, Networks, System};

pub struct Row {
    pub label: String,
    pub value: String,
    /// Names a person or a machine: host name, user name, addresses, paths.
    pub identifying: bool,
}

pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

pub const EXPORTS: &[&str] = &["Text", "JSON", "Markdown"];

/// `17,054,638,080` → `15.9 GiB`.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// `93784` → `1 day 2 h 3 min`.
pub fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs % 86_400 / 3600, secs % 3600 / 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(super::plural(d as usize, "day"));
    }
    if h > 0 {
        parts.push(format!("{h} h"));
    }
    parts.push(format!("{m} min"));
    parts.join(" ")
}

fn row(label: impl Into<String>, value: impl Into<String>) -> Row {
    Row { label: label.into(), value: value.into(), identifying: false }
}

fn private(label: impl Into<String>, value: impl Into<String>) -> Row {
    Row { label: label.into(), value: value.into(), identifying: true }
}

/// Read the machine's details. Takes a moment (it queries the OS for disks
/// and network interfaces), so call it off the UI thread.
pub fn collect() -> Vec<Section> {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    let unknown = || "Unknown".to_string();

    let mut system = vec![
        row("Operating system", System::long_os_version().or_else(System::name).unwrap_or_else(unknown)),
        row("Version", System::os_version().unwrap_or_else(unknown)),
        row("Kernel", System::kernel_version().unwrap_or_else(unknown)),
        row("Architecture", System::cpu_arch().unwrap_or_else(|| std::env::consts::ARCH.to_string())),
        private("Host name", System::host_name().unwrap_or_else(unknown)),
    ];
    if let Some(user) = std::env::var_os("USERNAME").or_else(|| std::env::var_os("USER")) {
        system.push(private("User", user.to_string_lossy()));
    }
    system.push(row("Uptime", duration(System::uptime())));
    if let Some(boot) = chrono::DateTime::from_timestamp(System::boot_time() as i64, 0) {
        system.push(row("Started", boot.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()));
    }
    if let Ok(zone) = iana_time_zone::get_timezone() {
        system.push(row("Time zone", zone));
    }

    let mut hardware = Vec::new();
    if let Some(cpu) = sys.cpus().first() {
        hardware.push(row("Processor", cpu.brand().trim()));
        if cpu.frequency() > 0 {
            hardware.push(row("Clock speed", format!("{:.2} GHz", cpu.frequency() as f64 / 1000.)));
        }
    }
    let logical = sys.cpus().len();
    hardware.push(row("Cores", match sys.physical_core_count() {
        Some(physical) if physical != logical => format!("{physical} physical, {logical} logical"),
        _ => logical.to_string(),
    }));
    hardware.push(row("Memory", format!("{} ({} available)", size(sys.total_memory()), size(sys.available_memory()))));
    if sys.total_swap() > 0 {
        hardware.push(row("Swap", format!("{} ({} used)", size(sys.total_swap()), size(sys.used_swap()))));
    }

    let storage = Disks::new_with_refreshed_list()
        .list()
        .iter()
        .map(|d| {
            let label = format!("{} ({})", d.mount_point().display(), d.file_system().to_string_lossy());
            row(label, format!("{} free of {}{}", size(d.available_space()), size(d.total_space()), if d.is_removable() { " · removable" } else { "" }))
        })
        .collect();

    let networks = Networks::new_with_refreshed_list();
    let mut interfaces: Vec<_> = networks.list().iter().collect();
    interfaces.sort_by(|a, b| a.0.cmp(b.0));
    let network = interfaces
        .into_iter()
        .filter(|(_, data)| !data.ip_networks().is_empty())
        .map(|(name, data)| {
            let mut ips: Vec<String> = data.ip_networks().iter().map(|n| format!("{}/{}", n.addr, n.prefix)).collect();
            ips.sort();
            private(name.as_str(), format!("{} · {}", data.mac_address(), ips.join(", ")))
        })
        .collect();

    let mut app = vec![row("SideKit", env!("CARGO_PKG_VERSION"))];
    if let Ok(exe) = std::env::current_exe() {
        app.push(private("Executable", exe.display().to_string()));
    }

    vec![
        Section { title: "System", rows: system },
        Section { title: "Hardware", rows: hardware },
        Section { title: "Storage", rows: storage },
        Section { title: "Network", rows: network },
        Section { title: "Application", rows: app },
    ]
}

/// The snapshot as `EXPORTS[format]`; `hide` leaves the identifying rows out.
pub fn export(sections: &[Section], hide: bool, format: usize) -> String {
    let kept: Vec<(&'static str, Vec<&Row>)> = sections
        .iter()
        .map(|s| (s.title, s.rows.iter().filter(|r| !(hide && r.identifying)).collect::<Vec<_>>()))
        .filter(|(_, rows)| !rows.is_empty())
        .collect();
    match format {
        1 => {
            let mut root = Map::new();
            for (title, rows) in kept {
                let section: Map<String, Value> = rows.iter().map(|r| (r.label.clone(), Value::String(r.value.clone()))).collect();
                root.insert(title.to_string(), section.into());
            }
            super::to_json(&root.into(), &super::Indent::Spaces(2))
        }
        2 => kept
            .iter()
            .map(|(title, rows)| {
                let table: String = rows.iter().map(|r| format!("| {} | {} |\n", r.label.replace('|', "\\|"), r.value.replace('|', "\\|"))).collect();
                format!("### {title}\n\n| | |\n| --- | --- |\n{table}")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => kept
            .iter()
            .map(|(title, rows)| {
                let width = rows.iter().map(|r| r.label.chars().count()).max().unwrap_or(0);
                let lines: String = rows.iter().map(|r| format!("  {:<width$}  {}\n", r.label, r.value)).collect();
                format!("{title}\n{lines}")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
    .trim_end()
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_and_durations() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(17_054_638_080), "15.9 GiB");
        assert_eq!(duration(59), "0 min");
        assert_eq!(duration(93_784), "1 day 2 h 3 min");
        assert_eq!(duration(2 * 86_400 + 60), "2 days 1 min");
    }

    fn sample() -> Vec<Section> {
        vec![
            Section { title: "System", rows: vec![row("Operating system", "Windows 11"), private("Host name", "ADA-PC")] },
            Section { title: "Network", rows: vec![private("Ethernet", "aa:bb · 10.0.0.2/24")] },
        ]
    }

    #[test]
    fn exports_and_hides() {
        let s = sample();
        assert_eq!(export(&s, false, 0), "System\n  Operating system  Windows 11\n  Host name         ADA-PC\n\nNetwork\n  Ethernet  aa:bb · 10.0.0.2/24");
        // With identifying fields hidden, a section left empty disappears too.
        assert_eq!(export(&s, true, 0), "System\n  Operating system  Windows 11");
        assert_eq!(export(&s, true, 1), "{\n  \"System\": {\n    \"Operating system\": \"Windows 11\"\n  }\n}");
        assert_eq!(export(&s, false, 2), "### System\n\n| | |\n| --- | --- |\n| Operating system | Windows 11 |\n| Host name | ADA-PC |\n\n### Network\n\n| | |\n| --- | --- |\n| Ethernet | aa:bb · 10.0.0.2/24 |");
        assert!(!export(&s, true, 2).contains("ADA-PC"));
    }

    #[test]
    fn reads_this_machine() {
        let sections = collect();
        assert_eq!(sections.iter().map(|s| s.title).collect::<Vec<_>>(), ["System", "Hardware", "Storage", "Network", "Application"]);
        let system = &sections[0].rows;
        assert!(system.iter().any(|r| r.label == "Host name" && r.identifying && !r.value.is_empty()));
        let cores = sections[1].rows.iter().find(|r| r.label == "Cores").unwrap();
        assert!(cores.value.chars().next().unwrap().is_ascii_digit());
        assert!(sections[1].rows.iter().any(|r| r.label == "Memory" && r.value.contains("available")));
        let hidden = export(&sections, true, 0);
        let host = &system.iter().find(|r| r.label == "Host name").unwrap().value;
        assert!(!hidden.contains("Host name") && (host.len() < 3 || !hidden.contains(host.as_str())));
    }
}
