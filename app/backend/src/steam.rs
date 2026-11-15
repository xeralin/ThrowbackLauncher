use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, Signal, System, UpdateKind};

use crate::env::{IS_WINDOWS, NOT_INSTALLED, Paths, SEASON_RUNNING, TL_LAUNCHER, home, resolve};
use crate::log::fail;
use crate::manifest::{edition_folder, hm_folder_name, is_installed, is_season_folder};
use crate::settings::LAUNCH_ARGS_UNCLOSED;

const PROTON_BUILTIN: [(&str, &str, &str); 2] = [
    (
        "Proton - Experimental",
        "proton_experimental",
        "Proton Experimental",
    ),
    ("Proton Hotfix", "proton_hotfix", "Proton Hotfix"),
];

const SYSTEM_COMPAT_DIRS: [&str; 2] = [
    "/usr/share/steam/compatibilitytools.d",
    "/usr/local/share/steam/compatibilitytools.d",
];

const STEAM_HOME_DIRS: [&str; 2] = [".local/share/Steam", ".steam/steam"];

const TERMINATE_GRACE: Duration = Duration::from_secs(2);
const KILL_GRACE: Duration = Duration::from_secs(3);
const EXIT_POLL: Duration = Duration::from_millis(20);
const COMPAT_DATA: &str = "STEAM_COMPAT_DATA_PATH";
const CLIENT_INSTALL: &str = "STEAM_COMPAT_CLIENT_INSTALL_PATH";
const COMMAND_MARK: &str = "%command%";

static GAME_EXE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:RainbowSix.*|LaunchR6)\.exe").expect("pattern"));

static COMPAT_TOOL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""compat_tools"\s*\{\s*"([^"]+)""#).expect("pattern"));

static DISPLAY_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""display_name"\s+"([^"]+)""#).expect("pattern"));

static LIBRARY_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""path"\s+"([^"]+)""#).expect("pattern"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proton {
    pub display: String,
    pub internal: String,
    pub binary: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtonEnv {
    pub prefix: PathBuf,
    pub steam: PathBuf,
}

impl ProtonEnv {
    pub fn vars(&self) -> [(&'static str, &Path); 2] {
        [(COMPAT_DATA, &self.prefix), (CLIENT_INSTALL, &self.steam)]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchCommand {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
}

struct Game {
    pid: u32,
    folder: String,
    env: Option<ProtonEnv>,
}

fn is_game(text: &OsStr) -> bool {
    GAME_EXE.is_match(&text.to_string_lossy())
}

fn scan(wanted: ProcessesToUpdate<'_>, kind: ProcessRefreshKind) -> Option<System> {
    let mut system = System::new().ok()?;
    system.refresh_processes_specifics(wanted, true, kind);
    Some(system)
}

fn candidates() -> Option<System> {
    let cmd = ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always);
    let listed = if IS_WINDOWS {
        ProcessRefreshKind::nothing()
    } else {
        cmd
    };
    let all = scan(ProcessesToUpdate::All, listed)?;
    let named = |process: &&Process| {
        if IS_WINDOWS {
            is_game(process.name())
        } else {
            process.cmd().iter().any(|argument| is_game(argument))
        }
    };
    let processes = all.processes().values();
    let pids: Vec<Pid> = processes.filter(named).map(Process::pid).collect();
    let detail = cmd
        .with_cwd(UpdateKind::Always)
        .with_environ(UpdateKind::Always);
    scan(ProcessesToUpdate::Some(&pids), detail)
}

fn variable(process: &Process, name: &str) -> Option<PathBuf> {
    process.environ().iter().find_map(|entry| {
        let entry = entry.to_string_lossy();
        let value = entry.strip_prefix(name)?.strip_prefix('=')?;
        (!value.is_empty()).then(|| PathBuf::from(value))
    })
}

fn game_env(process: &Process) -> Option<ProtonEnv> {
    Some(ProtonEnv {
        prefix: variable(process, COMPAT_DATA)?,
        steam: variable(process, CLIENT_INSTALL).unwrap_or_else(steam_dir),
    })
}

fn game_exe(process: &Process) -> Option<PathBuf> {
    let argument = process.cmd().iter().find(|argument| is_game(argument))?;
    let text = argument.to_string_lossy();
    let drive = text
        .strip_prefix(r"Z:\")
        .or_else(|| text.strip_prefix(r"z:\"));
    Some(match drive.filter(|_| !IS_WINDOWS) {
        Some(rest) => PathBuf::from(format!("/{}", rest.replace('\\', "/"))),
        None => PathBuf::from(argument),
    })
}

fn folder_below(path: &Path, roots: &[&PathBuf]) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let resolved = resolve(path);
    let below = |root: &&PathBuf| resolved.strip_prefix(root).ok()?.components().next();
    let folder = roots.iter().find_map(below)?;
    Some(folder.as_os_str().to_string_lossy().into_owned())
}

fn games(libraries: &[PathBuf]) -> Vec<Game> {
    let mut roots: Vec<&PathBuf> = libraries.iter().collect();
    roots.sort_by_key(|root| Reverse(root.components().count()));
    let system = candidates();
    let processes = system.iter().flat_map(|system| system.processes().values());
    let mut games: Vec<Game> = processes
        .filter_map(|process| {
            let env = game_env(process);
            if !IS_WINDOWS && env.is_none() {
                return None;
            }
            let paths = [game_exe(process), process.cwd().map(Path::to_path_buf)];
            let mut inside = paths.iter().flatten();
            let folder = inside.find_map(|path| folder_below(path, &roots))?;
            let pid = process.pid().as_u32();
            is_season_folder(Path::new(&folder)).then_some(Game { pid, folder, env })
        })
        .collect();
    games.sort_by_key(|game| game.pid);
    games
}

pub fn running_game_folders(libraries: &[PathBuf]) -> BTreeMap<String, Vec<u32>> {
    let mut folders = BTreeMap::<String, Vec<u32>>::new();
    for game in games(libraries) {
        folders.entry(game.folder).or_default().push(game.pid);
    }
    folders
}

pub fn running_game_env(libraries: &[PathBuf]) -> Option<ProtonEnv> {
    games(libraries).into_iter().find_map(|game| game.env)
}

pub fn is_game_running(libraries: &[PathBuf]) -> bool {
    !games(libraries).is_empty()
}

pub fn is_season_running(libraries: &[PathBuf], key: &str) -> bool {
    let hm = hm_folder_name(key);
    let games = games(libraries);
    games
        .iter()
        .any(|game| game.folder == key || game.folder == hm)
}

fn listed(pids: &[u32]) -> Option<System> {
    let pids: Vec<Pid> = pids.iter().map(|pid| Pid::from_u32(*pid)).collect();
    scan(
        ProcessesToUpdate::Some(&pids),
        ProcessRefreshKind::nothing(),
    )
}

pub fn game_started(pids: &[u32]) -> Option<i64> {
    let system = listed(pids)?;
    let started = system.processes().values().map(Process::start_time);
    started
        .filter(|started| *started > 0)
        .min()
        .map(u64::cast_signed)
}

pub fn prefix_pids(prefix: &Path) -> Vec<u32> {
    let wine = prefix.join("pfx");
    let holds = |process: &&Process| {
        variable(process, COMPAT_DATA).is_some_and(|path| path == prefix)
            || variable(process, "WINEPREFIX").is_some_and(|path| path == wine)
    };
    let environ = ProcessRefreshKind::nothing().with_environ(UpdateKind::Always);
    let system = scan(ProcessesToUpdate::All, environ);
    let processes = system.iter().flat_map(|system| system.processes().values());
    let inside = processes.filter(holds);
    inside.map(|process| process.pid().as_u32()).collect()
}

#[cfg(not(windows))]
fn exited(process: &Process) -> bool {
    use sysinfo::ProcessStatus::{Dead, Zombie};

    matches!(process.status(), Zombie | Dead)
}

#[cfg(windows)]
fn exited(process: &Process) -> bool {
    use std::os::windows::io::{AsRawHandle, HandleOrNull, OwnedHandle};

    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    let opened = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process.pid().as_u32()) };
    let Ok(handle) = OwnedHandle::try_from(unsafe { HandleOrNull::from_raw_handle(opened) }) else {
        return false;
    };
    unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) == WAIT_OBJECT_0 }
}

fn wait_gone(system: &mut System, targets: &mut Vec<(Pid, u64)>, grace: Duration) {
    let deadline = Instant::now() + grace;
    loop {
        let pids: Vec<Pid> = targets.iter().map(|(pid, _)| *pid).collect();
        let nothing = ProcessRefreshKind::nothing();
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, nothing);
        targets.retain(|(pid, started)| {
            let process = system.process(*pid);
            process.is_some_and(|process| process.start_time() == *started && !exited(process))
        });
        if targets.is_empty() || Instant::now() >= deadline {
            return;
        }
        thread::sleep(EXIT_POLL);
    }
}

pub fn stop_game(pids: &[u32]) {
    let Some(mut system) = listed(pids) else {
        return;
    };
    let mut targets = Vec::new();
    for process in system.processes().values() {
        let asked = process.kill_with(Signal::Term);
        if asked.unwrap_or_else(|| process.kill()) {
            targets.push((process.pid(), process.start_time()));
        }
    }
    wait_gone(&mut system, &mut targets, TERMINATE_GRACE);
    for process in targets.iter().filter_map(|(pid, _)| system.process(*pid)) {
        process.kill();
    }
    wait_gone(&mut system, &mut targets, KILL_GRACE);
}

fn steam_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for name in STEAM_HOME_DIRS {
        let root = resolve(&home.join(name));
        if root.is_dir() && !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

fn steam_dir() -> PathBuf {
    let home = home();
    let first = steam_roots(&home).into_iter().next();
    first.unwrap_or_else(|| home.join(STEAM_HOME_DIRS[0]))
}

fn sorted_entries(folder: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(folder).into_iter().flatten().flatten();
    let mut paths: Vec<PathBuf> = entries.map(|entry| entry.path()).collect();
    paths.sort();
    paths
}

fn steam_libraries(root: &Path) -> Vec<PathBuf> {
    let vdf = fs::read(root.join("steamapps").join("libraryfolders.vdf")).unwrap_or_default();
    let mut libraries = vec![root.to_owned()];
    for found in LIBRARY_PATH.captures_iter(&String::from_utf8_lossy(&vdf)) {
        let library = PathBuf::from(&found[1]);
        if !libraries.contains(&library) {
            libraries.push(library);
        }
    }
    libraries
}

fn proton_entry(folder: &Path) -> Option<Proton> {
    let binary = folder.join("proton");
    if !binary.exists() {
        return None;
    }
    let name = folder.file_name()?.to_string_lossy().into_owned();
    let builtin = PROTON_BUILTIN.iter().find(|(folder, _, _)| *folder == name);
    if let Some((_, internal, display)) = builtin {
        return Some(Proton {
            display: (*display).to_owned(),
            internal: (*internal).to_owned(),
            binary,
        });
    }
    let vdf = fs::read(folder.join("compatibilitytool.vdf")).unwrap_or_default();
    let text = String::from_utf8_lossy(&vdf);
    let tool = COMPAT_TOOL.captures(&text).map(|tool| tool[1].to_owned());
    let shown = tool.as_ref().and_then(|_| DISPLAY_NAME.captures(&text));
    let internal = tool.unwrap_or(name);
    Some(Proton {
        display: shown.map_or_else(|| internal.clone(), |shown| shown[1].to_owned()),
        internal,
        binary,
    })
}

fn protons(steam_roots: &[PathBuf], compat: &[PathBuf], own: &Path) -> Vec<Proton> {
    let mut folders = Vec::new();
    for library in steam_roots.iter().flat_map(|root| steam_libraries(root)) {
        let common = sorted_entries(&library.join("steamapps").join("common"));
        let named = |folder: &PathBuf| {
            let name = folder.file_name().unwrap_or_default();
            name.to_string_lossy().starts_with("Proton")
        };
        folders.extend(common.into_iter().filter(named));
    }
    let user = steam_roots
        .iter()
        .map(|root| root.join("compatibilitytools.d"));
    for folder in compat.iter().cloned().chain(user).chain([own.to_owned()]) {
        folders.extend(sorted_entries(&folder));
    }
    let mut protons: Vec<Proton> = Vec::new();
    for proton in folders.iter().filter_map(|folder| proton_entry(folder)) {
        if !protons
            .iter()
            .any(|known| known.internal == proton.internal)
        {
            protons.push(proton);
        }
    }
    protons
}

pub fn list_protons(paths: &Paths) -> Vec<Proton> {
    let extra = std::env::var_os("STEAM_EXTRA_COMPAT_TOOLS_PATHS").unwrap_or_default();
    let system = SYSTEM_COMPAT_DIRS.iter().map(PathBuf::from);
    let compat: Vec<PathBuf> = system.chain(std::env::split_paths(&extra)).collect();
    protons(&steam_roots(&home()), &compat, &paths.proton_dir())
}

pub fn resolve_proton<'a>(protons: &'a [Proton], season: &str, global: &str) -> Option<&'a Proton> {
    let builtin = PROTON_BUILTIN.map(|(_, internal, _)| internal);
    let mut choices = [season, global].into_iter().chain(builtin);
    let named = |choice| protons.iter().find(|proton| proton.internal == choice);
    choices.find_map(named).or(protons.last())
}

pub fn proton_env(prefix: &Path) -> io::Result<ProtonEnv> {
    fs::create_dir_all(prefix)?;
    Ok(ProtonEnv {
        prefix: prefix.to_owned(),
        steam: steam_dir(),
    })
}

fn assignment(option: &str) -> Option<(&str, &str)> {
    let (name, value) = option.split_once('=')?;
    let identifier = name.starts_with(|first: char| first == '_' || first.is_alphabetic())
        && name
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric());
    identifier.then_some((name, value))
}

pub fn expand_launch_args(options: &str, command: &[String]) -> Result<LaunchCommand> {
    let split = |text: &str| shlex::split(text).context(LAUNCH_ARGS_UNCLOSED);
    let mut env = BTreeMap::new();
    if !options.contains(COMMAND_MARK) {
        let argv = command.iter().cloned().chain(split(options)?).collect();
        return Ok(LaunchCommand { argv, env });
    }
    let wrapped = shlex::try_join(command.iter().map(String::as_str))?;
    let mut words = split(&options.replace(COMMAND_MARK, &wrapped))?
        .into_iter()
        .peekable();
    while let Some((name, value)) = words.peek().and_then(|word| assignment(word)) {
        env.insert(name.to_owned(), value.to_owned());
        words.next();
    }
    let argv: Vec<String> = words.collect();
    ensure!(!argv.is_empty(), "Launch options leave no command to run");
    Ok(LaunchCommand { argv, env })
}

pub fn prune_prefixes(paths: &Paths, libraries: &[PathBuf]) {
    let Ok(entries) = fs::read_dir(paths.prefix_dir()) else {
        return;
    };
    if !libraries.iter().all(|root| root.exists()) || is_game_running(libraries) {
        return;
    }
    let prefixes: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    for prefix in prefixes.iter().filter(|prefix| prefix.is_dir()) {
        let key = prefix.file_name().unwrap_or_default().to_string_lossy();
        let hm = hm_folder_name(&key);
        let used = |root: &PathBuf| root.join(&*key).is_dir() || root.join(&hm).is_dir();
        if !libraries.iter().any(used) {
            let _ = fs::remove_dir_all(prefix);
        }
    }
}

pub fn uninstall(paths: &Paths, libraries: &[PathBuf], key: &str, hm: bool) -> Result<()> {
    let folder = edition_folder(key, hm);
    let folders = libraries.iter().map(|root| root.join(&folder));
    let installs: Vec<PathBuf> = folders.filter(|path| is_installed(path)).collect();
    let prefix = paths.prefix_dir().join(key);
    if installs.is_empty() && !prefix.exists() {
        bail!(NOT_INSTALLED);
    }
    if is_season_running(libraries, key) {
        bail!(SEASON_RUNNING);
    }
    for path in &installs {
        let removed =
            fs::remove_file(path.join(TL_LAUNCHER)).and_then(|()| fs::remove_dir_all(path));
        if let Err(error) = removed {
            bail!(fail(
                "File removal failed",
                format!("{error}: '{}'", path.display())
            ));
        }
    }
    let other = edition_folder(key, !hm);
    if prefix.exists()
        && !libraries.iter().any(|root| root.join(&other).is_dir())
        && let Err(error) = fs::remove_dir_all(&prefix)
    {
        bail!(fail(
            "Proton prefix removal failed",
            format!("{error}: '{}'", prefix.display())
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::log::take_recorded;
    use crate::throwbackloader::testing::names;

    const KEY: &str = "Y5S3_ShadowLegacy";
    const COMMAND: [&str; 3] = [
        "/home/u/My Steam/Proton - Experimental/proton",
        "run",
        "/lib/Y5S3_ShadowLegacy/LaunchR6.exe",
    ];
    const GE_VDF: &str = "\"compatibilitytools\"\n{\n  \"compat_tools\"\n  {\n    \"Proton-GE\"\n    {\n      \"install_path\" \".\"\n      \"display_name\" \"GE-Proton9-20\"\n      \"from_oslist\"  \"windows\"\n    }\n  }\n}\n";
    const PLAIN_VDF: &str =
        "\"compatibilitytools\"\n{\n\t\"compat_tools\"\n\t{\n\t\t\"plain\"\n\t\t{\n\t\t}\n\t}\n}\n";

    struct Stage {
        _dir: TempDir,
        root: PathBuf,
        paths: Paths,
        libraries: Vec<PathBuf>,
    }

    impl Stage {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = resolve(dir.path());
            let paths = Paths::new(root.join("asset"), root.join("data"));
            let libraries = vec![root.join("one"), root.join("two")];
            for folder in libraries.iter().chain([&paths.data_root]) {
                fs::create_dir_all(folder).unwrap();
            }
            Self {
                _dir: dir,
                root,
                paths,
                libraries,
            }
        }

        fn season(&self, library: usize, name: &str) -> PathBuf {
            let folder = self.libraries[library].join(name);
            fs::create_dir_all(folder.join("datapc64")).unwrap();
            fs::write(folder.join(TL_LAUNCHER), b"").unwrap();
            fs::write(folder.join("datapc64").join("a.forge"), b"x").unwrap();
            folder
        }

        fn prefix(&self, name: &str) -> PathBuf {
            let prefix = self.paths.prefix_dir().join(name);
            fs::create_dir_all(prefix.join("pfx").join("drive_c")).unwrap();
            prefix
        }
    }

    fn proton(folder: &Path, vdf: Option<&str>) {
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join("proton"), b"").unwrap();
        if let Some(vdf) = vdf {
            fs::write(folder.join("compatibilitytool.vdf"), vdf).unwrap();
        }
    }

    #[test]
    fn game_processes_are_told_by_their_exe_name() {
        for name in [
            "RainbowSix.exe",
            "RainbowSix_Vulkan.exe",
            "RainbowSix_DX11.exe",
            "RainbowSixGame.exe",
            "LaunchR6.exe",
            r"Z:\lib\Y1S0_Vanilla\RainbowSix.exe",
        ] {
            assert!(is_game(OsStr::new(name)), "{name}");
        }
        for name in [
            "rainbowsix.exe",
            "RainbowSix",
            "LaunchR7.exe",
            "Liberator.exe",
        ] {
            assert!(!is_game(OsStr::new(name)), "{name}");
        }
    }

    #[cfg(unix)]
    mod live {
        use std::io::{BufRead, BufReader};
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        use std::process::{Child, Command, Stdio};

        use chrono::Utc;

        use super::*;

        struct Fake(Child);

        impl Fake {
            fn start(argv0: &Path, cwd: &Path, variables: &[(&str, &Path)]) -> Self {
                let mut command = Command::new("sleep");
                let sleep = command.arg0(argv0).arg("30").current_dir(cwd);
                let plain = sleep.env_remove(COMPAT_DATA).env_remove("WINEPREFIX");
                Self(plain.envs(variables.iter().copied()).spawn().unwrap())
            }

            fn pid(&self) -> u32 {
                self.0.id()
            }
        }

        impl Drop for Fake {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        #[test]
        fn proton_games_inside_the_libraries_count_as_running() {
            let stage = Stage::new();
            let prefix = stage.prefix("Y99S1_Probe");
            let steam = stage.root.join("steam");
            let proton = [(COMPAT_DATA, prefix.as_path()), (CLIENT_INSTALL, &steam)];
            let exe = stage.season(0, "Y99S1_Probe").join("LaunchR6.exe");
            let first = Fake::start(&exe, &stage.root, &proton);
            let second = Fake::start(&exe, &stage.root, &proton);
            let bare = stage.season(0, "Y99S2_NoProton").join("LaunchR6.exe");
            let _bare = Fake::start(&bare, &stage.root, &[]);
            let outside = stage.root.join("elsewhere").join("Y99S3_Outside");
            let _outside = Fake::start(&outside.join("LaunchR6.exe"), &stage.root, &proton);
            let tools = stage.libraries[0].join("Tools").join("LaunchR6.exe");
            let _tools = Fake::start(&tools, &stage.root, &proton);
            let heated = stage.season(1, "Y99S4_HeatedMetal").join("RainbowSix.exe");
            let heated = Fake::start(&heated, &stage.root, &proton);
            let by_cwd = stage.season(1, "Y99S5_ByCwd");
            let by_cwd = Fake::start(Path::new("LaunchR6.exe"), &by_cwd, &proton);
            let wine = stage.season(0, "Y99S6_Wine").join("RainbowSix_Vulkan.exe");
            let wine = format!("Z:{}", wine.to_str().unwrap().replace('/', "\\"));
            let wine = Fake::start(Path::new(&wine), &stage.root, &proton);
            let idle = stage.season(1, "Y99S7_Idle");
            let _idle = Fake::start(Path::new("sleep"), &idle, &proton);

            let mut twice = vec![first.pid(), second.pid()];
            twice.sort_unstable();
            let expected = BTreeMap::from([
                ("Y99S1_Probe".to_owned(), twice),
                ("Y99S4_HeatedMetal".to_owned(), vec![heated.pid()]),
                ("Y99S5_ByCwd".to_owned(), vec![by_cwd.pid()]),
                ("Y99S6_Wine".to_owned(), vec![wine.pid()]),
            ]);
            assert_eq!(running_game_folders(&stage.libraries), expected);
            assert!(is_game_running(&stage.libraries));
            assert!(is_season_running(&stage.libraries, "Y99S1_Probe"));
            assert!(is_season_running(&stage.libraries, "Y99S4_Anything"));
            assert!(!is_season_running(&stage.libraries, "Y99S2_NoProton"));
            assert!(!is_season_running(&stage.libraries, "Y99S3_Outside"));
            assert!(!is_game_running(&[]));
            assert_eq!(
                running_game_env(&stage.libraries),
                Some(ProtonEnv { prefix, steam })
            );
            assert_eq!(running_game_env(&[stage.root.join("nowhere")]), None);
        }

        #[test]
        fn nested_libraries_resolve_against_the_deepest_one() {
            let stage = Stage::new();
            let prefix = stage.prefix("Y99S10_Nested");
            let proton = [(COMPAT_DATA, prefix.as_path())];
            let deep = stage.libraries[0].join("deep");
            let nested = deep.join("Y99S10_Nested").join("LaunchR6.exe");
            let nested = Fake::start(&nested, &stage.root, &proton);
            let shallow = stage.libraries[0].join("Y99S11_Shallow");
            let shallow = Fake::start(&shallow.join("LaunchR6.exe"), &stage.root, &proton);
            let expected = BTreeMap::from([
                ("Y99S10_Nested".to_owned(), vec![nested.pid()]),
                ("Y99S11_Shallow".to_owned(), vec![shallow.pid()]),
            ]);
            let libraries = [stage.libraries[0].clone(), deep];
            assert_eq!(running_game_folders(&libraries), expected);
        }

        #[test]
        fn prefix_processes_are_found_by_their_environment() {
            let stage = Stage::new();
            let prefix = stage.prefix("Y99S8_Prefix");
            let wineprefix = prefix.join("pfx");
            let other = stage.prefix("Y99S8_Other");
            let start = |name: &str, value: &Path| {
                Fake::start(Path::new("sleep"), &stage.root, &[(name, value)])
            };
            let compat = start(COMPAT_DATA, &prefix);
            let wine = start("WINEPREFIX", &wineprefix);
            let _swapped = start("WINEPREFIX", &prefix);
            let _other = start(COMPAT_DATA, &other);
            let mut found = prefix_pids(&prefix);
            found.sort_unstable();
            let mut wanted = vec![compat.pid(), wine.pid()];
            wanted.sort_unstable();
            assert_eq!(found, wanted);
            assert!(prefix_pids(&stage.paths.prefix_dir().join("Y99S8_None")).is_empty());
        }

        #[test]
        fn the_start_time_is_the_earliest_process_start() {
            let stage = Stage::new();
            let before = Utc::now().timestamp();
            let first = Fake::start(Path::new("sleep"), &stage.root, &[]);
            let second = Fake::start(Path::new("sleep"), &stage.root, &[]);
            let early = game_started(&[first.pid()]).unwrap();
            let late = game_started(&[second.pid()]).unwrap();
            assert!(before - 2 <= early && early <= late && late <= Utc::now().timestamp() + 1);
            assert_eq!(game_started(&[second.pid(), first.pid()]), Some(early));
            assert_eq!(game_started(&[u32::MAX, second.pid(), 0]), Some(late));
            assert_eq!(game_started(&[]), None);
            assert_eq!(game_started(&[u32::MAX]), None);
        }

        #[test]
        fn stop_terminates_first_and_kills_what_ignores_it() {
            let mut polite = Command::new("sleep").arg("30").spawn().unwrap();
            let mut deaf = Command::new("sh");
            deaf.args(["-c", "trap '' TERM; echo ready; exec sleep 30"]);
            let mut deaf = deaf.stdout(Stdio::piped()).spawn().unwrap();
            let mut ready = String::new();
            let mut output = BufReader::new(deaf.stdout.take().unwrap());
            output.read_line(&mut ready).unwrap();
            let started = Instant::now();
            stop_game(&[polite.id(), 0, deaf.id(), u32::MAX]);
            let elapsed = started.elapsed();
            assert!(elapsed >= TERMINATE_GRACE, "{elapsed:?}");
            assert!(elapsed < TERMINATE_GRACE + KILL_GRACE, "{elapsed:?}");
            assert_eq!(polite.wait().unwrap().signal(), Some(libc::SIGTERM));
            assert_eq!(deaf.wait().unwrap().signal(), Some(libc::SIGKILL));
        }

        #[test]
        fn stop_returns_as_soon_as_everything_is_gone() {
            let spawn = || Command::new("sleep").arg("30").spawn().unwrap();
            let mut games: Vec<Child> = (0..3).map(|_| spawn()).collect();
            let pids: Vec<u32> = games.iter().map(Child::id).collect();
            let started = Instant::now();
            stop_game(&pids);
            stop_game(&pids);
            for game in &mut games {
                assert_eq!(game.wait().unwrap().signal(), Some(libc::SIGTERM));
            }
            stop_game(&pids);
            stop_game(&[]);
            assert!(started.elapsed() < TERMINATE_GRACE);
        }

        #[test]
        fn a_running_season_blocks_uninstall_and_prune() {
            let stage = Stage::new();
            let folder = stage.season(0, "Y99S9_Running");
            let prefix = stage.prefix("Y99S9_Running");
            let stale = stage.prefix("Y98S1_Stale");
            let exe = folder.join("LaunchR6.exe");
            let game = Fake::start(&exe, &stage.root, &[(COMPAT_DATA, &prefix)]);
            let blocked = uninstall(&stage.paths, &stage.libraries, "Y99S9_Running", false);
            assert_eq!(blocked.unwrap_err().to_string(), "Stop this season first");
            prune_prefixes(&stage.paths, &stage.libraries);
            assert!(exe.exists() && stale.is_dir());
            drop(game);
            prune_prefixes(&stage.paths, &stage.libraries);
            assert!(!stale.exists() && prefix.is_dir());
            uninstall(&stage.paths, &stage.libraries, "Y99S9_Running", false).unwrap();
            assert!(!folder.exists() && !prefix.exists());
            assert!(take_recorded().is_empty());
        }
    }

    #[test]
    fn steam_roots_are_the_existing_home_folders_without_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let home = resolve(dir.path());
        let native = home.join(".local").join("share").join("Steam");
        let linked = home.join(".steam").join("steam");
        assert!(steam_roots(&home).is_empty());
        fs::create_dir_all(&linked).unwrap();
        assert_eq!(steam_roots(&home), std::slice::from_ref(&linked));
        fs::create_dir_all(&native).unwrap();
        assert_eq!(steam_roots(&home), [native.clone(), linked.clone()]);
        #[cfg(unix)]
        {
            fs::remove_dir(&linked).unwrap();
            std::os::unix::fs::symlink(&native, &linked).unwrap();
            assert_eq!(steam_roots(&home), [native]);
        }
    }

    #[test]
    fn protons_are_listed_in_valves_scan_order_and_the_first_name_wins() {
        let dir = tempfile::tempdir().unwrap();
        let root = resolve(dir.path());
        let steam = root.join("steam");
        let second = root.join("second library");
        let common = steam.join("steamapps").join("common");
        fs::create_dir_all(&common).unwrap();
        fs::write(
            steam.join("steamapps").join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                steam.display(),
                second.display()
            ),
        )
        .unwrap();
        proton(&common.join("Proton 9.0 (Beta)"), None);
        proton(&common.join("Proton - Experimental"), None);
        proton(&common.join("SteamLinuxRuntime_sniper"), None);
        fs::create_dir_all(common.join("Proton Broken")).unwrap();
        let far = second.join("steamapps").join("common");
        proton(&far.join("Proton Hotfix"), None);
        proton(&far.join("Proton 9.0 (Beta)"), None);
        let system = root.join("system");
        proton(&system.join("proton-ge-custom"), Some(GE_VDF));
        let extra = root.join("extra");
        proton(&extra.join("Tool"), Some(GE_VDF));
        proton(&extra.join("NoDisplay"), Some(PLAIN_VDF));
        let user = steam.join("compatibilitytools.d");
        proton(&user.join("GE-Proton9-1"), None);
        fs::write(user.join("readme.txt"), b"").unwrap();
        let own = root.join("data").join("bin").join("proton");
        proton(&own.join("Proton-R6HM"), None);
        proton(&own.join("Proton Hotfix"), None);

        let compat = [system.clone(), root.join("missing"), extra];
        let listed = protons(std::slice::from_ref(&steam), &compat, &own);
        let shown: Vec<(&str, &str)> = listed
            .iter()
            .map(|proton| (proton.internal.as_str(), proton.display.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("proton_experimental", "Proton Experimental"),
                ("Proton 9.0 (Beta)", "Proton 9.0 (Beta)"),
                ("proton_hotfix", "Proton Hotfix"),
                ("Proton-GE", "GE-Proton9-20"),
                ("plain", "plain"),
                ("GE-Proton9-1", "GE-Proton9-1"),
                ("Proton-R6HM", "Proton-R6HM"),
            ]
        );
        assert_eq!(
            listed[1].binary,
            common.join("Proton 9.0 (Beta)").join("proton")
        );
        assert_eq!(listed[2].binary, far.join("Proton Hotfix").join("proton"));
        assert_eq!(
            listed[3].binary,
            system.join("proton-ge-custom").join("proton")
        );
        assert!(protons(&[], &[], &root.join("none")).is_empty());
    }

    #[test]
    fn the_proton_choice_falls_back_step_by_step() {
        fn pick<'a>(protons: &'a [Proton], season: &str, global: &str) -> Option<&'a str> {
            resolve_proton(protons, season, global).map(|proton| proton.internal.as_str())
        }
        let named = |internal: &str| Proton {
            display: internal.to_uppercase(),
            internal: internal.to_owned(),
            binary: PathBuf::from(format!("/p/{internal}/proton")),
        };
        let all = ["ge", "proton_hotfix", "proton_experimental", "old"].map(named);
        assert_eq!(pick(&[], "", ""), None);
        assert_eq!(pick(&all, "", ""), Some("proton_experimental"));
        assert_eq!(pick(&all[..2], "", ""), Some("proton_hotfix"));
        assert_eq!(pick(&all[..1], "", ""), Some("ge"));
        assert_eq!(pick(&all, "old", "ge"), Some("old"));
        assert_eq!(pick(&all, "removed", "ge"), Some("ge"));
        assert_eq!(pick(&all, "", "ge"), Some("ge"));
        assert_eq!(pick(&all[1..], "", "ge"), Some("proton_experimental"));
    }

    #[test]
    fn the_proton_environment_names_the_prefix_and_creates_it() {
        let stage = Stage::new();
        let prefix = stage.paths.prefix_dir().join(KEY);
        let env = proton_env(&prefix).unwrap();
        assert!(prefix.is_dir());
        assert_eq!(
            env.vars(),
            [
                ("STEAM_COMPAT_DATA_PATH", prefix.as_path()),
                ("STEAM_COMPAT_CLIENT_INSTALL_PATH", env.steam.as_path())
            ]
        );
        fs::write(stage.root.join("file"), b"").unwrap();
        assert!(proton_env(&stage.root.join("file").join("prefix")).is_err());
    }

    #[test]
    fn launch_options_wrap_the_command_and_set_variables() {
        let command = COMMAND.map(str::to_owned);
        let expand = |options: &str| {
            let launch = expand_launch_args(options, &command);
            launch.map(|launch| (launch.argv, launch.env)).ok()
        };
        let around = |before: &[&str], after: &[&str]| -> Vec<String> {
            let parts = before.iter().chain(&COMMAND).chain(after);
            parts.map(|part| (*part).to_owned()).collect()
        };
        let pairs = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            let owned = |(name, value): &(&str, &str)| ((*name).to_owned(), (*value).to_owned());
            pairs.iter().map(owned).collect()
        };
        assert_eq!(expand(""), Some((around(&[], &[]), pairs(&[]))));
        assert_eq!(
            expand("-windowed  'a b' \"c d\" e\\ f"),
            Some((around(&[], &["-windowed", "a b", "c d", "e f"]), pairs(&[])))
        );
        assert_eq!(
            expand("PROTON_LOG=1"),
            Some((around(&[], &["PROTON_LOG=1"]), pairs(&[])))
        );
        assert_eq!(
            expand("PROTON_LOG=1 %command%"),
            Some((around(&[], &[]), pairs(&[("PROTON_LOG", "1")])))
        );
        assert_eq!(
            expand("PROTON_LOG=1 DXVK_HUD='fps,frametimes' gamemoderun %command% -dx11"),
            Some((
                around(&["gamemoderun"], &["-dx11"]),
                pairs(&[("PROTON_LOG", "1"), ("DXVK_HUD", "fps,frametimes")])
            ))
        );
        assert_eq!(
            expand("_B= X=a=b 9C=3 A=1 %command% Y=1"),
            Some((
                around(&["9C=3", "A=1"], &["Y=1"]),
                pairs(&[("_B", ""), ("X", "a=b")])
            ))
        );
        let inner = format!("exec '{}' run {}", COMMAND[0], COMMAND[2]);
        assert_eq!(
            expand("bash -c \"exec %command%\""),
            Some((
                ["bash", "-c", &inner].map(str::to_owned).to_vec(),
                pairs(&[])
            ))
        );
        assert_eq!(expand("-name \"open"), None);
        assert_eq!(expand("# gamemoderun %command%"), None);
    }

    #[test]
    fn prune_removes_only_prefixes_without_a_game_folder() {
        let stage = Stage::new();
        fs::create_dir_all(stage.libraries[0].join("Y1S0_Vanilla")).unwrap();
        fs::create_dir_all(stage.libraries[1].join("Y5S3_HeatedMetal")).unwrap();
        fs::write(stage.libraries[1].join("Y2S1_VelvetShell"), b"").unwrap();
        prune_prefixes(&stage.paths, &stage.libraries);
        assert!(!stage.paths.prefix_dir().exists());
        for name in ["Y1S0_Vanilla", KEY, "Y2S1_VelvetShell"] {
            stage.prefix(name);
        }
        fs::write(stage.paths.prefix_dir().join("note.txt"), b"").unwrap();
        prune_prefixes(&stage.paths, &stage.libraries);
        assert_eq!(
            names(&stage.paths.prefix_dir()),
            ["Y1S0_Vanilla", KEY, "note.txt"]
        );
        stage.prefix("Y3S1_Chimera");
        fs::remove_dir_all(&stage.libraries[1]).unwrap();
        prune_prefixes(&stage.paths, &stage.libraries);
        assert_eq!(names(&stage.paths.prefix_dir()).len(), 4);
        prune_prefixes(&stage.paths, &stage.libraries[..1]);
        assert_eq!(
            names(&stage.paths.prefix_dir()),
            ["Y1S0_Vanilla", "note.txt"]
        );
    }

    #[test]
    fn uninstall_removes_the_edition_and_a_prefix_no_edition_needs() {
        let stage = Stage::new();
        let run = |hm: bool| {
            let done = uninstall(&stage.paths, &stage.libraries, KEY, hm);
            done.map_err(|error| error.to_string())
        };
        assert_eq!(run(false), Err("Not installed".to_owned()));
        let first = stage.season(0, KEY);
        let second = stage.season(1, KEY);
        let partial = stage.libraries[0].join("Y1S0_Vanilla");
        fs::create_dir_all(&partial).unwrap();
        let heated = stage.libraries[1].join("Y5S3_HeatedMetal");
        fs::create_dir_all(&heated).unwrap();
        let prefix = stage.prefix(KEY);
        assert_eq!(run(false), Ok(()));
        assert!(!first.exists() && !second.exists());
        assert!(partial.is_dir() && heated.is_dir() && prefix.is_dir());
        assert_eq!(run(true), Ok(()));
        assert!(heated.is_dir() && !prefix.exists());
        assert_eq!(run(true), Err("Not installed".to_owned()));
        fs::write(heated.join(TL_LAUNCHER), b"").unwrap();
        stage.prefix(KEY);
        stage.season(0, KEY);
        assert_eq!(run(true), Ok(()));
        assert!(!heated.exists() && prefix.is_dir() && first.is_dir());
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn a_failed_removal_is_logged_with_the_folder_it_hit() {
        let stage = Stage::new();
        let run = || {
            let done = uninstall(&stage.paths, &stage.libraries, KEY, false);
            done.unwrap_err().to_string()
        };
        let folder = stage.libraries[1].join(KEY);
        fs::create_dir_all(folder.join(TL_LAUNCHER)).unwrap();
        assert_eq!(run(), "File removal failed");
        fs::remove_dir_all(&folder).unwrap();
        let prefix = stage.paths.prefix_dir().join(KEY);
        fs::create_dir_all(stage.paths.prefix_dir()).unwrap();
        fs::write(&prefix, b"").unwrap();
        assert_eq!(run(), "Proton prefix removal failed");
        let logged = take_recorded();
        assert_eq!(logged.len(), 2, "{logged:?}");
        assert!(logged[0].contains(folder.to_str().unwrap()), "{logged:?}");
        assert!(logged[1].contains(prefix.to_str().unwrap()), "{logged:?}");
    }
}
