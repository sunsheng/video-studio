//! 已验证 workflow 的参数注入。
//!
//! 官方模板一律只读，项目内保存的是**已经真机验证过的 API 格式**基线，
//! 每份基线自带一段 `_studio.bindings`，说明逐镜头参数该写到哪个节点的哪个输入上。
//! 这样加一个新模型系列只需要放一份基线，不需要改代码。
//!
//! 基线缺失或绑定对不上时报 `model_contract_violation` 并停下——
//! **不允许静默换成别的模型或别的节点**。

use serde_json::{Map, Value};
use studio_core::{Result, StudioError};

/// 一份已验证基线。
#[derive(Debug, Clone)]
pub struct Workflow {
    /// API 格式的节点图，已剥掉 `_studio`。
    graph: Value,
    /// 参数名 → 若干个 `<节点 id>.inputs.<输入名>` 路径。
    bindings: Map<String, Value>,
    name: String,
    /// 绑定是否已经真机跑通核验过。未核验的基线不允许用来渲染——
    /// 绑错节点会静默产出错的画面，比直接报错难查得多。
    verified: bool,
    source: Option<String>,
}

impl Workflow {
    /// 从基线目录加载 `<family>/<name>.json`，例如 `minimax_h3/t2v`。
    pub fn load(dir: &std::path::Path, workflow: &str) -> Result<Workflow> {
        let path = dir.join(format!("{workflow}.json"));
        let text =
            std::fs::read_to_string(&path).map_err(|_| StudioError::ModelContractViolation {
                detail: format!(
                    "找不到已验证基线 {}。提示词包里写的 workflow 必须是基线目录里存在的那些",
                    path.display()
                ),
            })?;
        Workflow::parse(&text, workflow)
    }

    pub fn parse(text: &str, name: &str) -> Result<Workflow> {
        let mut v: Value =
            serde_json::from_str(text).map_err(|e| StudioError::ModelContractViolation {
                detail: format!("基线 {name} 不是合法 JSON：{e}"),
            })?;
        let obj = v
            .as_object_mut()
            .ok_or_else(|| StudioError::ModelContractViolation {
                detail: format!("基线 {name} 的顶层必须是对象"),
            })?;

        let meta = obj.remove("_studio").unwrap_or(Value::Null);
        let bindings = meta
            .get("bindings")
            .and_then(|b| b.as_object())
            .cloned()
            .ok_or_else(|| StudioError::ModelContractViolation {
                detail: format!("基线 {name} 缺少 _studio.bindings，无法确定参数写到哪个节点"),
            })?;

        // API 格式的判据：每个值都带 class_type 和 inputs。
        for (id, node) in obj.iter() {
            if node.get("class_type").is_none() || node.get("inputs").is_none() {
                return Err(StudioError::ModelContractViolation {
                    detail: format!(
                        "基线 {name} 的节点 {id} 不是 API 格式。UI workflow 不能直接提交，\
                         需要用当前 ComfyUI 版本导出 API 图"
                    ),
                });
            }
        }

        let verified = meta
            .get("bindings_verified")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let source = meta
            .get("source")
            .and_then(|s| s.as_str())
            .map(String::from);
        Ok(Workflow {
            graph: v,
            bindings,
            name: name.to_string(),
            verified,
            source,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parameters(&self) -> Vec<String> {
        self.bindings.keys().cloned().collect()
    }

    pub fn is_verified(&self) -> bool {
        self.verified
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// 用来渲染之前必须核验过。
    pub fn require_verified(&self) -> Result<()> {
        if self.verified {
            return Ok(());
        }
        Err(StudioError::ModelContractViolation {
            detail: format!(
                "基线 {} 的参数绑定尚未核验。绑错节点会静默产出错的画面，\
                 所以在真机跑通并把 _studio.bindings_verified 改成 true 之前不允许用它渲染",
                self.name
            ),
        })
    }

    /// 逐条检查绑定指向的节点确实存在。真机跑之前先把打字错误挡掉。
    pub fn check(&self) -> Result<()> {
        let mut probe = Map::new();
        for k in self.bindings.keys() {
            probe.insert(k.clone(), Value::Null);
        }
        self.apply(&probe).map(|_| ())
    }

    /// 把逐镜头参数写进图里，返回可直接提交给 `/prompt` 的副本。
    ///
    /// 基线里没有绑定的参数会被忽略（不同系列支持的参数本来就不同），
    /// 但绑定指向的节点不存在是硬错误——那说明基线自己坏了。
    pub fn apply(&self, params: &Map<String, Value>) -> Result<Value> {
        let mut graph = self.graph.clone();
        for (key, targets) in &self.bindings {
            let Some(value) = params.get(key) else {
                continue;
            };
            let Some(list) = targets.as_array() else {
                return Err(self.broken(format!("绑定 {key} 必须是路径数组")));
            };
            for t in list {
                let path = t
                    .as_str()
                    .ok_or_else(|| self.broken(format!("绑定 {key} 里有非字符串路径")))?;
                self.write_at(&mut graph, path, value.clone())?;
            }
        }
        Ok(graph)
    }

    fn write_at(&self, graph: &mut Value, path: &str, value: Value) -> Result<()> {
        let mut parts = path.split('.');
        let (Some(node), Some("inputs"), Some(input)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(self.broken(format!("路径 {path} 应当形如 <节点id>.inputs.<输入名>")));
        };
        if parts.next().is_some() {
            return Err(self.broken(format!("路径 {path} 层级过深")));
        }
        let target = graph
            .get_mut(node)
            .and_then(|n| n.get_mut("inputs"))
            .and_then(|i| i.as_object_mut())
            .ok_or_else(|| self.broken(format!("基线里没有节点 {node} 或它没有 inputs")))?;
        target.insert(input.to_string(), value);
        Ok(())
    }

    fn broken(&self, detail: String) -> StudioError {
        StudioError::ModelContractViolation {
            detail: format!("基线 {} 有问题：{detail}", self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn baseline() -> String {
        json!({
            "_studio": { "bindings": {
                "positive": ["6.inputs.text"],
                "negative": ["7.inputs.text"],
                "width":  ["5.inputs.width"],
                "height": ["5.inputs.height"],
                "length_frames": ["5.inputs.length"],
                "seed": ["3.inputs.seed"]
            }},
            "3": { "class_type": "KSampler",       "inputs": { "seed": 0, "steps": 20 } },
            "5": { "class_type": "EmptyLatentVideo","inputs": { "width": 512, "height": 512, "length": 16 } },
            "6": { "class_type": "CLIPTextEncode",  "inputs": { "text": "" } },
            "7": { "class_type": "CLIPTextEncode",  "inputs": { "text": "" } }
        })
        .to_string()
    }

    fn params() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("positive".into(), json!("船头掠过清透湖面"));
        m.insert("negative".into(), json!("文字, 水印"));
        m.insert("width".into(), json!(1080));
        m.insert("height".into(), json!(1920));
        m.insert("length_frames".into(), json!(42));
        m.insert("seed".into(), json!(101001));
        m.insert("fps".into(), json!(30)); // 基线没绑定，应被忽略
        m
    }

    #[test]
    fn parameters_land_on_the_bound_nodes() {
        let w = Workflow::parse(&baseline(), "minimax_h3/t2v").unwrap();
        let g = w.apply(&params()).unwrap();
        assert_eq!(g["6"]["inputs"]["text"], json!("船头掠过清透湖面"));
        assert_eq!(g["7"]["inputs"]["text"], json!("文字, 水印"));
        assert_eq!(g["5"]["inputs"]["width"], json!(1080));
        assert_eq!(g["5"]["inputs"]["length"], json!(42));
        assert_eq!(g["3"]["inputs"]["seed"], json!(101001));
        // 没绑定的参数不会凭空塞进图里
        assert!(g["3"]["inputs"].get("fps").is_none());
        // 基线里原有的其它输入原样保留
        assert_eq!(g["3"]["inputs"]["steps"], json!(20));
        // _studio 不会被提交出去
        assert!(g.get("_studio").is_none());
    }

    #[test]
    fn applying_does_not_mutate_the_baseline() {
        let w = Workflow::parse(&baseline(), "x").unwrap();
        w.apply(&params()).unwrap();
        let again = w.apply(&Map::new()).unwrap();
        assert_eq!(
            again["5"]["inputs"]["width"],
            json!(512),
            "基线必须保持原样，逐镜头之间不能串味"
        );
    }

    /// 绑错节点会静默产出错的画面。未核验的基线宁可报错也不渲染。
    #[test]
    fn an_unverified_baseline_refuses_to_render() {
        let w = Workflow::parse(&baseline(), "wan2_2/i2v").unwrap();
        assert!(!w.is_verified(), "没写 bindings_verified 就默认未核验");
        let e = w.require_verified().unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.message().contains("尚未核验"));

        let mut v: Value = serde_json::from_str(&baseline()).unwrap();
        v["_studio"]["bindings_verified"] = json!(true);
        let w = Workflow::parse(&v.to_string(), "minimax_h3/t2v").unwrap();
        assert!(w.is_verified());
        w.require_verified().unwrap();
    }

    #[test]
    fn check_catches_a_typo_in_a_binding_path() {
        let bad = json!({
            "_studio": { "bindings": { "seed": ["3.inputs.seed"], "width": ["nope.inputs.width"] } },
            "3": { "class_type": "KSampler", "inputs": { "seed": 0 } }
        })
        .to_string();
        let w = Workflow::parse(&bad, "b").unwrap();
        let e = w.check().unwrap_err();
        assert!(e.message().contains("没有节点 nope"));
    }

    #[test]
    fn check_passes_on_a_sound_baseline() {
        Workflow::parse(&baseline(), "ok").unwrap().check().unwrap();
    }

    #[test]
    fn a_missing_baseline_is_a_contract_violation_not_a_fallback() {
        let d = tempfile::tempdir().unwrap();
        let e = Workflow::load(d.path(), "minimax_h3/t2v").unwrap_err();
        assert_eq!(e.code(), "model_contract_violation");
        assert!(e.remedy().contains("不允许静默替换"));
    }

    #[test]
    fn ui_workflow_is_rejected_with_an_explanation() {
        let ui = json!({
            "_studio": { "bindings": {} },
            "nodes": [{ "id": 1, "type": "KSampler" }]
        })
        .to_string();
        let e = Workflow::parse(&ui, "bad").unwrap_err();
        assert!(e.message().contains("API 格式"));
    }

    #[test]
    fn missing_bindings_are_rejected() {
        let e = Workflow::parse(
            &json!({ "1": { "class_type": "X", "inputs": {} } }).to_string(),
            "b",
        )
        .unwrap_err();
        assert!(e.message().contains("bindings"));
    }

    #[test]
    fn a_binding_pointing_at_a_missing_node_is_a_broken_baseline() {
        let bad = json!({
            "_studio": { "bindings": { "seed": ["99.inputs.seed"] } },
            "3": { "class_type": "KSampler", "inputs": { "seed": 0 } }
        })
        .to_string();
        let w = Workflow::parse(&bad, "b").unwrap();
        let mut p = Map::new();
        p.insert("seed".into(), json!(1));
        let e = w.apply(&p).unwrap_err();
        assert!(e.message().contains("没有节点 99"));
    }
}
