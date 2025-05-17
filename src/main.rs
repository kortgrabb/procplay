use chrono::{DateTime, Local};
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::{collections::HashMap, env, fs, path::PathBuf, thread, time::Duration};

#[derive(Deserialize)]
struct Config {
    tracked: Vec<String>,
}

fn load_config() -> Config {
    let mut path = dirs::config_dir().expect("no config dir");
    path.push("playtime-tracker/config.yaml");

    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap()).expect("failed to create config dir");
        let default_config = r#"
tracked:
    - "example_program"
"#;
        fs::write(&path, default_config).expect("failed to create config file");
        println!("Created default config at {:?}", path);
        std::process::exit(0);
    }

    let config_str = fs::read_to_string(&path).expect("failed to read config file");
    let config: Config = serde_yaml::from_str(&config_str).expect("failed to parse config");

    if config.tracked.is_empty() {
        println!("No programs to track in config. Please add them.");
        std::process::exit(0);
    }
    config
}

fn init_db() -> Connection {
    let mut db_path = dirs::data_local_dir().expect("no data dir");
    db_path.push("playtime-tracker");
    db_path.set_extension("sqlite");
    
    if !db_path.exists() {
        fs::create_dir_all(db_path.parent().unwrap()).expect("failed to create data dir");
        println!("Created data dir at {:?}", db_path);
    }

    let conn = Connection::open(db_path).expect("failed to open db");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id       INTEGER PRIMARY KEY,
            path     TEXT NOT NULL,
            pid      INTEGER NOT NULL,
            started  TEXT NOT NULL,
            ended    TEXT
        );",
    )
    .unwrap();

    conn
}

fn run_daemon(config: &Config, conn: &Connection) {
    // map pid -> (binary name, start timestamp)
    let mut active: HashMap<i32, (String, DateTime<Local>)> = HashMap::new();
    loop {
        let mut seen_pids = Vec::new();
        for entry in fs::read_dir("/proc").unwrap() {
            if let Ok(ent) = entry {
                if let Ok(pid) = ent.file_name().to_string_lossy().parse::<i32>() {
                    let mut comm = PathBuf::from("/proc");
                    comm.push(pid.to_string());
                    comm.push("comm");
                    if let Ok(name) = fs::read_to_string(&comm) {
                        let name = name.trim().to_string();
                        if config.tracked.contains(&name) {
                            seen_pids.push((pid, name));
                        }
                    }
                }
            }
        }

        // detect new
        for (pid, name) in &seen_pids {
            if !active.contains_key(pid) {
                let now = Local::now();
                conn.execute(
                    "INSERT INTO sessions (path, pid, started) VALUES (?1, ?2, ?3)",
                    params![name, pid, now.to_rfc3339()],
                )
                .unwrap();
                active.insert(*pid, (name.clone(), now));
                println!("Started {} (pid {}) at {}", name, pid, now);
            }
        }
        // detect ended
        let prev_pids: Vec<i32> = active.keys().cloned().collect();
        for pid in prev_pids {
            if !seen_pids.iter().any(|(p, _)| *p == pid) {
                if let Some((name, start)) = active.remove(&pid) {
                    let now = Local::now();
                    conn.execute(
                        "UPDATE sessions SET ended = ?1 WHERE pid = ?2 AND ended IS NULL",
                        params![now.to_rfc3339(), pid],
                    )
                    .unwrap();
                    println!("Ended {} (pid {}) at {}", name, pid, now);
                }
            }
        }

        thread::sleep(Duration::from_secs(1));
    }
}

fn report(conn: &Connection) {
    let mut stmt = conn
        .prepare(
            "SELECT path, SUM(
            strftime('%s', ended) - strftime('%s', started)
        ) AS total_secs
        FROM sessions
        WHERE ended IS NOT NULL
        GROUP BY path;",
        )
        .unwrap();

    let mut rows = stmt.query([]).unwrap();
    println!("Playtime report:");
    while let Ok(Some(row)) = rows.next() {
        let path: String = row.get(0).unwrap();
        let secs: i64 = row.get(1).unwrap();
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        println!("- {}: {}h {}m {}s", path, h, m, s);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let config = load_config();
    let conn = init_db();

    if args.len() > 1 && args[1] == "report" {
        report(&conn);
    } else {
        run_daemon(&config, &conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_config() {
        let config = load_config();
        assert!(!config.tracked.is_empty());
    }

    #[test]
    fn test_init_db() {
        let conn = init_db();
        assert!(conn.execute("SELECT 1", []).is_ok());
    }
}