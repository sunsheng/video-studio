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
    #[serde(default = "default_node")]
    pub node: String,
    /// 旧的多节点写法。**保留这个字段只为了不静默丢掉它**——serde 会把未知
    /// 字段直接吞掉，于是升级后 `[comfy].nodes = [...]` 的老部署会安静地
    /// 回落到本机默认端口，渲染打到一个根本不对的地方。认下来、取第一个，
    /// 并让 `doctor` 报出来，跟 `COMFY_NODES` 环境变量那条路对称。
    #[serde(default)]
    pub nodes: Option<Vec<String>>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    /// 一次往 ComfyUI 队列里压多少个镜头。入口只有一个 URL，客户端看不见
    /// 后端有几个节点，所以并发度只能显式给。默认 16——队列深一点没有坏处，
    /// 排队的部分由那一侧调度，而队列太浅会让后端节点闲着。
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// preview 挂不挂 turbo LoRA。默认开——预览门要看的只是构图与内容，
    /// 4/8 步比 20 步快得多。片段库里没有对应的叠加层、或者它还没真机核验时
    /// 会自动退回普通组合并在进度里说明，所以开着也不会悄悄跑出不可信的东西。
    #[serde(default = "default_preview_turbo")]
    pub preview_turbo: bool,
}

fn default_preview_turbo() -> bool {
    true
}

fn default_node() -> String {
    "http://127.0.0.1:9001".to_string()
}
fn default_timeout() -> u64 {
    1800
}
fn default_poll() -> u64 {
    3
}
fn default_concurrency() -> usize {
    16
}

impl Default for ComfyConfig {
    fn default() -> Self {
        ComfyConfig {
            node: default_node(),
            nodes: None,
            timeout_secs: default_timeout(),
            poll_interval_secs: default_poll(),
            concurrency: default_concurrency(),
            preview_turbo: default_preview_turbo(),
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
        }
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

    /// ComfyUI 的入口地址。**只有一个**——多节点的调度是那一侧的事
    /// （通常是个负载均衡代理），控制面不再维护节点集合。
    ///
    /// `COMFY_NODES`（复数）是旧名，仍然认，但只取第一个值；写了多个时
    /// [`Settings::comfy_node_legacy_extras`] 会把被忽略的那些报出来，
    /// 由 `doctor` 提示改名，不静默丢弃。
    pub fn comfy_node(&self) -> String {
        self.comfy_node_values()
            .into_iter()
            .next()
            .unwrap_or_else(|| self.file.comfy.node.trim_end_matches('/').to_string())
    }

    /// 被忽略的旧多节点配置——`COMFY_NODES` 里除第一个之外的值，以及
    /// `config.toml` 的 `[comfy].nodes`（只在环境变量没给时才轮到它）。
    /// 空表示没有配置被忽略。
    pub fn comfy_node_legacy_extras(&self) -> Vec<String> {
        let from_env = self.comfy_env_values();
        if !from_env.is_empty() {
            return from_env.into_iter().skip(1).collect();
        }
        // 环境变量没给，才看 TOML 里的旧写法。第一个已经被 comfy_node() 用上了。
        self.comfy_toml_legacy_values()
            .into_iter()
            .skip(1)
            .collect()
    }

    fn comfy_node_values(&self) -> Vec<String> {
        let from_env = self.comfy_env_values();
        if !from_env.is_empty() {
            return from_env;
        }
        self.comfy_toml_legacy_values()
    }

    fn comfy_env_values(&self) -> Vec<String> {
        self.env
            .get("COMFY_NODE")
            .or_else(|| self.env.get("COMFY_NODES"))
            .map(|v| split_nodes(v))
            .unwrap_or_default()
    }

    /// `config.toml` 里旧的 `[comfy].nodes`。不认它的话 serde 会把这个字段
    /// 静默吞掉，老部署升级后会安静地回落到本机默认端口。
    fn comfy_toml_legacy_values(&self) -> Vec<String> {
        self.file
            .comfy
            .nodes
            .as_deref()
            .map(|list| {
                list.iter()
                    .map(|s| s.trim().trim_end_matches('/').to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 接入代理时的 Bearer token。没配就不带 `Authorization` 头——
    /// 直连一个没有鉴权的 ComfyUI 时本来就不需要。
    pub fn comfy_token(&self) -> Option<String> {
        self.env
            .get("COMFY_TOKEN")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn comfy_timeout_secs(&self) -> u64 {
        self.env
            .get("COMFY_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(self.file.comfy.timeout_secs)
    }

    /// 同时在途的镜头数，至少 1。
    pub fn comfy_concurrency(&self) -> usize {
        self.env
            .get("COMFY_CONCURRENCY")
            .and_then(|v| v.parse().ok())
            .unwrap_or(self.file.comfy.concurrency)
            .max(1)
    }

    /// preview 是否挂 turbo LoRA。`COMFY_PREVIEW_TURBO=0` 可以关掉。
    pub fn comfy_preview_turbo(&self) -> bool {
        match self.env.get("COMFY_PREVIEW_TURBO") {
            Some(v) => !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            ),
            None => self.file.comfy.preview_turbo,
        }
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

fn split_nodes(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
        assert_eq!(s.comfy_node(), "http://bundle:9001");
    }

    /// 进程环境里可能已经有 `COMFY_NODE(S)`（云端会话就是这么配的），
    /// 那种情况下这个测试问的「没有任何配置时的默认值」问不出来，跳过。
    #[test]
    fn comfy_node_falls_back_to_the_local_default() {
        if std::env::var("COMFY_NODE").is_ok() || std::env::var("COMFY_NODES").is_ok() {
            return;
        }
        assert_eq!(
            Settings::load(None, None).comfy_node(),
            "http://127.0.0.1:9001"
        );
    }

    #[test]
    fn comfy_node_is_preferred_over_the_legacy_plural_name() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(
            bundle.path().join(".env"),
            "COMFY_NODES=http://old:9001\nCOMFY_NODE=http://new:9001\n",
        )
        .unwrap();
        let s = Settings::load(None, Some(bundle.path()));
        assert_eq!(s.comfy_node(), "http://new:9001");
        assert!(s.comfy_node_legacy_extras().is_empty());
    }

    /// 旧的复数名写了多个地址时只用第一个，但被忽略的那些要能报出来——
    /// 静默丢掉配置正是这个项目最不能接受的失败方式。
    #[test]
    fn extra_values_in_the_legacy_plural_name_are_reported_not_dropped() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(
            bundle.path().join(".env"),
            "COMFY_NODES=http://a:9001,http://b:9002,http://c:9003\n",
        )
        .unwrap();
        let s = Settings::load(None, Some(bundle.path()));
        assert_eq!(s.comfy_node(), "http://a:9001");
        assert_eq!(
            s.comfy_node_legacy_extras(),
            vec!["http://b:9002", "http://c:9003"]
        );
    }

    #[test]
    fn trailing_slashes_are_normalised() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(bundle.path().join(".env"), "COMFY_NODE=http://a:9001//\n").unwrap();
        let s = Settings::load(None, Some(bundle.path()));
        assert_eq!(s.comfy_node(), "http://a:9001");
    }

    /// `config.toml` 里旧的 `[comfy].nodes` 不能被 serde 静默吞掉——那会让老部署
    /// 升级后安静地回落到本机默认端口，渲染打到一个根本不对的地方。
    #[test]
    fn a_legacy_toml_node_list_is_used_and_reported_not_silently_dropped() {
        if std::env::var("COMFY_NODE").is_ok() || std::env::var("COMFY_NODES").is_ok() {
            return; // 进程环境里已有配置时，问不出「只有 TOML」这个场景
        }
        let prog = tempfile::tempdir().unwrap();
        std::fs::write(
            prog.path().join("config.toml"),
            "[comfy]\nnodes = [\"http://old-a:9001\", \"http://old-b:9002\"]\n",
        )
        .unwrap();
        let s = Settings::load(Some(prog.path()), None);
        assert_eq!(
            s.comfy_node(),
            "http://old-a:9001",
            "旧的 TOML 节点列表要接着用第一个，而不是回落到本机默认端口"
        );
        assert_eq!(s.comfy_node_legacy_extras(), vec!["http://old-b:9002"]);
    }

    /// 环境变量优先于 TOML 的旧写法，而且此时不该再去报 TOML 里那些。
    #[test]
    fn the_env_var_wins_over_the_legacy_toml_list() {
        let prog = tempfile::tempdir().unwrap();
        std::fs::write(
            prog.path().join("config.toml"),
            "[comfy]\nnodes = [\"http://old:9001\"]\n",
        )
        .unwrap();
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(bundle.path().join(".env"), "COMFY_NODE=http://new:9001\n").unwrap();
        let s = Settings::load(Some(prog.path()), Some(bundle.path()));
        assert_eq!(s.comfy_node(), "http://new:9001");
        assert!(s.comfy_node_legacy_extras().is_empty());
    }

    #[test]
    fn comfy_token_is_none_when_unset_or_blank() {
        let bundle = tempfile::tempdir().unwrap();
        std::fs::write(bundle.path().join(".env"), "COMFY_TOKEN=  \n").unwrap();
        assert!(Settings::load(None, Some(bundle.path()))
            .comfy_token()
            .is_none());

        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join(".env"), "COMFY_TOKEN=abc123\n").unwrap();
        assert_eq!(
            Settings::load(None, Some(other.path()))
                .comfy_token()
                .as_deref(),
            Some("abc123")
        );
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
