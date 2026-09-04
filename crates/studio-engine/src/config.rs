//! 配置解析。
//!
//! 外部程序（ffmpeg / ffprobe）**不要求在 PATH 中**。查找顺序：
//!
//! 1. bundle 的 `.env`
//! 2. 程序目录的 `.env`
//! 3. 进程环境变量
//! 4. `config.toml` 的 `[media]` / `[comfy]` 段
//! 5. PATH
//!
//! 靠前的覆盖靠后的。这样同一台机器上不同作品可以指向不同的 ffmpeg 或不同的
//! ComfyUI 集群，而不必改全局配置。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileConfig {
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub comfy: ComfyConfig,
    #[serde(default)]
    pub model: ModelConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaConfig {
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComfyConfig {
    #[serde(default = "default_nodes")]
    pub nodes: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
}

fn default_nodes() -> Vec<String> {
    (9001..=9008)
        .map(|p| format!("http://127.0.0.1:{p}"))
        .collect()
}
fn default_timeout() -> u64 {
    1800
}
fn default_poll() -> u64 {
    3
}

impl Default for ComfyConfig {
    fn default() -> Self {
        ComfyConfig {
            nodes: default_nodes(),
            timeout_secs: default_timeout(),
            poll_interval_secs: default_poll(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default = "default_family")]
    pub core_family: String,
}

fn default_family() -> String {
    "minimax_h3".to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            core_family: default_family(),
        }
    }
}

/// 解析后的有效配置。
#[derive(Debug, Clone)]
pub struct Settings {
    pub file: FileConfig,
    /// 按优先级合并后的 KEY=VALUE。
    pub env: BTreeMap<String, String>,
    /// 找过的位置，用于 `tool_unavailable` 的报错。
    pub searched: Vec<String>,
    /// 本次进程临时排除的 ComfyUI 节点——来自 `studio.comfy.exclude_node`，
    /// 不写 `.env`、不落盘，只在当次会话内生效。
    excluded_comfy_nodes: Vec<String>,
}

impl Settings {
    /// `bundle_root` 为 None 时只读程序目录与进程环境（例如 `studiod doctor` 不在作品里跑）。
    pub fn load(program_dir: Option<&Path>, bundle_root: Option<&Path>) -> Settings {
        let mut env: BTreeMap<String, String> = BTreeMap::new();
        let mut searched = Vec::new();

        // 优先级从低到高写入，后写的覆盖先写的。
        for (k, v) in std::env::vars() {
            if is_studio_key(&k) {
                env.insert(k, v);
            }
        }
        searched.push("进程环境变量".to_string());

        if let Some(dir) = program_dir {
            let p = dir.join(".env");
            if let Some(map) = read_dotenv(&p) {
                env.extend(map);
            }
            searched.push(format!("{}", p.display()));
        }
        if let Some(root) = bundle_root {
            let p = root.join(".env");
            if let Some(map) = read_dotenv(&p) {
                env.extend(map);
            }
            searched.push(format!("{}", p.display()));
        }

        let mut file = FileConfig::default();
        if let Some(dir) = program_dir {
            let p = dir.join("config.toml");
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(parsed) = toml::from_str::<FileConfig>(&text) {
                    file = parsed;
                }
                searched.push(format!("{}", p.display()));
            }
        }
        searched.push("PATH".to_string());

        Settings {
            file,
            env,
            searched,
            excluded_comfy_nodes: Vec::new(),
        }
    }

    /// 叠加临时排除的节点。返回自身以便链式调用。
    pub fn exclude_comfy_nodes(mut self, excluded: impl IntoIterator<Item = String>) -> Self {
        self.excluded_comfy_nodes = excluded.into_iter().collect();
        self
    }

    /// 解析一个外部程序的位置。返回 None 表示到处都找不到。
    pub fn tool_path(&self, tool: &str) -> Option<PathBuf> {
        let key = format!("{}_PATH", tool.to_uppercase());
        if let Some(v) = self.env.get(&key) {
            let p = PathBuf::from(v);
            if p.is_file() {
                return Some(p);
            }
        }
        let from_file = match tool {
            "ffmpeg" => self.file.media.ffmpeg_path.as_ref(),
            "ffprobe" => self.file.media.ffprobe_path.as_ref(),
            _ => None,
        };
        if let Some(v) = from_file {
            let p = PathBuf::from(v);
            if p.is_file() {
                return Some(p);
            }
        }
        which(tool)
    }

    pub fn comfy_nodes(&self) -> Vec<String> {
        let all = if let Some(v) = self.env.get("COMFY_NODES") {
            let list: Vec<String> = v
                .split(',')
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !list.is_empty() {
                list
            } else {
                self.default_comfy_nodes()
            }
        } else {
            self.default_comfy_nodes()
        };
        if self.excluded_comfy_nodes.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|n| !self.excluded_comfy_nodes.contains(n))
            .collect()
    }

    fn default_comfy_nodes(&self) -> Vec<String> {
        self.file
            .comfy
            .nodes
            .iter()
            .map(|s| s.trim_end_matches('/').to_string())
            .collect()
    }

    pub fn comfy_timeout_secs(&self) -> u64 {
        self.env
            .get("COMFY_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(self.file.comfy.timeout_secs)
    }

    pub fn comfy_poll_secs(&self) -> u64 {
        self.env
            .get("COMFY_POLL_INTERVAL_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(self.file.comfy.poll_interval_secs)
    }

    pub fn core_model_family(&self) -> String {
        self.env
            .get("CORE_MODEL_FAMILY")
            .cloned()
            .unwrap_or_else(|| self.file.model.core_family.clone())
    }
}

fn is_studio_key(k: &str) -> bool {
    k.ends_with("_PATH") && (k.starts_with("FFMPEG") || k.starts_with("FFPROBE"))
        || k.starts_with("COMFY_")
        || k == "CORE_MODEL_FAMILY"
}

/// 极简 .env 解析：`KEY=VALUE`，`#` 注释，值两侧的引号会被剥掉。
pub fn read_dotenv(path: &Path) -> Option<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(v);
        map.insert(k.to_string(), v.to_string());
    }
    Some(map)
}

fn which(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(tool);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_handles_comments_quotes_and_export() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".env");
        std::fs::write(
            &p,
            "# 注释\n\nFFMPEG_PATH=\"/opt/bin/ffmpeg\"\nexport FFPROBE_PATH='/opt/bin/ffprobe'\nCOMFY_NODES=http://a:9001, http://b:9002/\n空行后还有=值\n",
        )
        .unwrap();
        let m = read_dotenv(&p).unwrap();
        assert_eq!(m.get("FFMPEG_PATH").unwrap(), "/opt/bin/ffmpeg");
        assert_eq!(m.get("FFPROBE_PATH").unwrap(), "/opt/bin/ffprobe");
        assert!(m.contains_key("COMFY_NODES"));
    }

    #[test]
    fn bundle_env_beats_program_env() {
        let prog = tempfile::tempdir().unwrap();
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(
            prog.path().join(".env"),
            "COMFY_NODES=http://program:9001\n",
        )
        .unwrap();
        std::fs::write(
            bundle.path().join(".env"),
            "COMFY_NODES=http://bundle:9001\n",
        )
        .unwrap();
        let s = Settings::load(Some(prog.path()), Some(bundle.path()));
        assert_eq!(s.comfy_nodes(), vec!["http://bundle:9001"]);
    }

    #[test]
    fn comfy_nodes_default_to_the_local_eight() {
        let s = Settings::load(None, None);
        let nodes = s.comfy_nodes();
        assert_eq!(nodes.len(), 8);
        assert_eq!(nodes[0], "http://127.0.0.1:9001");
        assert_eq!(nodes[7], "http://127.0.0.1:9008");
    }

    #[test]
    fn excluded_nodes_are_filtered_out_of_comfy_nodes() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(
            bundle.path().join(".env"),
            "COMFY_NODES=http://a:9001,http://b:9002,http://c:9003\n",
        )
        .unwrap();
        let s = Settings::load(None, Some(bundle.path()))
            .exclude_comfy_nodes(["http://b:9002".to_string()]);
        assert_eq!(s.comfy_nodes(), vec!["http://a:9001", "http://c:9003"]);
    }

    #[test]
    fn trailing_slashes_are_normalised() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(
            bundle.path().join(".env"),
            "COMFY_NODES=http://a:9001/,http://b:9002//\n",
        )
        .unwrap();
        let s = Settings::load(None, Some(bundle.path()));
        assert_eq!(s.comfy_nodes(), vec!["http://a:9001", "http://b:9002"]);
    }

    #[test]
    fn tool_path_prefers_env_over_path() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("ffmpeg");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();
        std::fs::write(
            dir.path().join(".env"),
            format!("FFMPEG_PATH={}\n", fake.display()),
        )
        .unwrap();
        let s = Settings::load(None, Some(dir.path()));
        assert_eq!(s.tool_path("ffmpeg").unwrap(), fake);
    }

    #[test]
    fn tool_path_is_none_when_configured_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "FFMPEG_PATH=/definitely/not/here/ffmpeg\n",
        )
        .unwrap();
        let s = Settings::load(None, Some(dir.path()));
        // 配错了路径就当作没配，继续退回 PATH；本机没装 ffmpeg 时应为 None。
        if which("ffmpeg").is_none() {
            assert!(s.tool_path("ffmpeg").is_none());
        }
    }

    #[test]
    fn config_toml_supplies_media_paths() {
        let prog = tempfile::tempdir().unwrap();
        let fake = prog.path().join("ffprobe");
        std::fs::write(&fake, "").unwrap();
        std::fs::write(
            prog.path().join("config.toml"),
            format!("[media]\nffprobe_path = \"{}\"\n", fake.display()),
        )
        .unwrap();
        let s = Settings::load(Some(prog.path()), None);
        assert_eq!(s.tool_path("ffprobe").unwrap(), fake);
    }
}
