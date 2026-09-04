//! 状态持久化。
//!
//! 一个 bundle 一个 SQLite 库，放在 `.studio/studio.db`。没有 `run_id`——
//! 库里只有一部作品，所以每张表都不需要项目维度。
//!
//! `.studio/` 是服务端私有的。Agent 不该碰它，但「不该」不等于「不能」，
//! 所以这里维护一个覆盖 stages 与 questions 的摘要：外部改动会在下一次
//! 读取时以 [`StudioError::StateDrift`] 暴露出来，而不是静默生效。

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use studio_core::contract::{AnswerOption, Question, SelectionType};
use studio_core::{Event, LoadedStage, Outputs, Result, StageId, StageState, StudioError};

const SCHEMA_VERSION: i64 = 1;

pub struct Store {
    conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store(.studio/studio.db)")
    }
}

fn oops(e: impl std::fmt::Display) -> StudioError {
    StudioError::internal(e.to_string())
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Store {
    /// 打开已有库并校验完整性。
    pub fn open(path: &std::path::Path) -> Result<Store> {
        let conn = Connection::open(path).map_err(oops)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(oops)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(oops)?;
        // WAL 允许并发读，但写锁一次只能有一个连接拿到——同一个 bundle
        // 上可能同时有主连接和确定性阶段 worker 各自开着一个连接，
        // 忙锁不设超时的话，撞车时会立即报 SQLITE_BUSY 而不是排队等一下。
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(oops)?;
        let store = Store { conn };
        store.verify_integrity()?;
        Ok(store)
    }

    /// 新建库并写入初始状态：第一个阶段处于草稿。
    pub fn create(path: &std::path::Path, title: &str, program_version: &str) -> Result<Store> {
        let conn = Connection::open(path).map_err(oops)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(oops)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(oops)?;
        conn.execute_batch(
            r#"
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE stages (
                stage        TEXT PRIMARY KEY,
                state        TEXT NOT NULL,
                attempt      INTEGER NOT NULL,
                outputs_json TEXT,
                summary      TEXT,
                updated_at   TEXT NOT NULL
            );
            CREATE TABLE questions (
                question_id    TEXT PRIMARY KEY,
                stage          TEXT NOT NULL,
                prompt         TEXT NOT NULL,
                selection_type TEXT NOT NULL,
                options_json   TEXT NOT NULL,
                status         TEXT NOT NULL,
                answer         TEXT,
                created_at     TEXT NOT NULL,
                answered_at    TEXT
            );
            CREATE TABLE events (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                at      TEXT NOT NULL,
                stage   TEXT NOT NULL,
                kind    TEXT NOT NULL,
                summary TEXT NOT NULL,
                error   TEXT
            );
            CREATE TABLE artifacts (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                stage     TEXT NOT NULL,
                kind      TEXT NOT NULL,
                rel_path  TEXT NOT NULL,
                meta_json TEXT,
                at        TEXT NOT NULL
            );
            CREATE TABLE integrity (id INTEGER PRIMARY KEY CHECK (id = 1), digest TEXT NOT NULL);
            -- 撤销栈：每个改变状态的操作在动手前压一份快照，undo 弹出最上面那份。
            -- 就是编辑器的 Ctrl+Z，不是版本管理——没有命名版本，也没有历史列表。
            CREATE TABLE undo_stack (
                seq            INTEGER PRIMARY KEY AUTOINCREMENT,
                label          TEXT NOT NULL,
                taken_at       TEXT NOT NULL,
                stages_json    TEXT NOT NULL,
                questions_json TEXT NOT NULL
            );
            "#,
        )
        .map_err(oops)?;

        let store = Store { conn };
        let t = now();
        for (k, v) in [
            ("title", title),
            ("program_version", program_version),
            ("schema_version", &SCHEMA_VERSION.to_string()),
            ("created_at", &t),
        ] {
            store
                .conn
                .execute(
                    "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                    params![k, v],
                )
                .map_err(oops)?;
        }
        // 全部阶段先落成草稿，状态机之后只做迁移不做创建。
        for stage in StageId::all() {
            store
                .conn
                .execute(
                    "INSERT INTO stages (stage, state, attempt, outputs_json, summary, updated_at)
                     VALUES (?1, ?2, 1, NULL, NULL, ?3)",
                    params![stage.as_str(), StageState::Draft.as_str(), t],
                )
                .map_err(oops)?;
        }
        store
            .conn
            .execute("INSERT INTO integrity (id, digest) VALUES (1, '')", [])
            .map_err(oops)?;
        store.reseal()?;
        Ok(store)
    }

    // ---------- meta ----------

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(oops)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(oops)?;
        Ok(())
    }

    pub fn title(&self) -> Result<String> {
        Ok(self
            .meta("title")?
            .unwrap_or_else(|| "未命名作品".to_string()))
    }

    // ---------- stages ----------

    pub fn load_stage(&self, stage: StageId) -> Result<LoadedStage> {
        let (state, attempt, outputs_json): (String, u32, Option<String>) = self
            .conn
            .query_row(
                "SELECT state, attempt, outputs_json FROM stages WHERE stage = ?1",
                params![stage.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StudioError::StateDrift {
                    detail: format!("阶段 {stage} 的记录不存在"),
                },
                other => oops(other),
            })?;

        let state = match state.as_str() {
            "draft" => StageState::Draft,
            "awaiting_confirmation" => StageState::AwaitingConfirmation,
            "approved" => StageState::Approved,
            other => {
                return Err(StudioError::StateDrift {
                    detail: format!("阶段 {stage} 的状态值非法：{other}"),
                })
            }
        };

        let outputs = match outputs_json {
            None => None,
            Some(s) => Some(parse_outputs(&s, stage)?),
        };
        let question = self.question_for(stage)?;
        LoadedStage::load(stage, state, attempt, outputs, question)
    }

    /// 落一个阶段的状态。带门时同时写入/更新确认问题。
    pub fn save_stage(
        &self,
        stage: StageId,
        state: StageState,
        attempt: u32,
        outputs: Option<&Outputs>,
        summary: Option<&str>,
        question: Option<&Question>,
    ) -> Result<()> {
        let outputs_json = match outputs {
            Some(o) => Some(serde_json::to_string(o).map_err(oops)?),
            None => None,
        };

        // stages 表的更新、questions 表的增删、reseal() 的完整性摘要，
        // 必须在同一个事务里原子生效。这三步是分开的 SQL 语句——不包一个
        // 事务的话，另一个连接（比如确定性阶段的后台 worker 和主线程各自
        // 开着的连接）就有窗口能读到「阶段已经是 awaiting_confirmation，
        // 但确认门还没写进 questions 表」这种半写状态，直接触发
        // state_drift。以前所有写 AwaitingConfirmation 的调用都来自
        // Agent 的同步 submit_stage，读写在同一个连接、同一个调用栈里，
        // 这个窗口不会被外部观察到；preview 这种由后台 worker 自己产出
        // 确认门的阶段，读写发生在两个连接上，窗口就能被撞见。
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(oops)?;
        let result: Result<()> = (|| {
            self.conn
                .execute(
                    "UPDATE stages SET state = ?2, attempt = ?3, outputs_json = ?4, summary = ?5, updated_at = ?6
                     WHERE stage = ?1",
                    params![stage.as_str(), state.as_str(), attempt, outputs_json, summary, now()],
                )
                .map_err(oops)?;

            match (state, question) {
                (StageState::AwaitingConfirmation, Some(q)) => self.put_question(q)?,
                // 离开挂起状态时，门随之消失——不留「已取消」的残骸。
                _ => self.clear_question(stage)?,
            }
            self.reseal()
        })();

        match result {
            Ok(()) => self.conn.execute_batch("COMMIT").map_err(oops),
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn stage_summary(&self, stage: StageId) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT summary FROM stages WHERE stage = ?1",
                params![stage.as_str()],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(oops)
            .map(|o| o.flatten())
    }

    // ---------- questions ----------

    fn put_question(&self, q: &Question) -> Result<()> {
        let options = serde_json::to_string(&q.options).map_err(oops)?;
        let selection = match q.selection_type {
            SelectionType::Single => "single",
            SelectionType::Multi => "multi",
        };
        // 同一个门重新挂起时整行覆盖，answer 一并清空。稳定的 question_id 不变。
        self.conn
            .execute(
                "INSERT INTO questions
                   (question_id, stage, prompt, selection_type, options_json, status, answer, created_at, answered_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, ?6, NULL)
                 ON CONFLICT(question_id) DO UPDATE SET
                   prompt = excluded.prompt,
                   selection_type = excluded.selection_type,
                   options_json = excluded.options_json,
                   status = 'pending',
                   answer = NULL,
                   created_at = excluded.created_at,
                   answered_at = NULL",
                params![q.question_id, q.stage.as_str(), q.prompt, selection, options, now()],
            )
            .map_err(oops)?;
        Ok(())
    }

    fn clear_question(&self, stage: StageId) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM questions WHERE stage = ?1",
                params![stage.as_str()],
            )
            .map_err(oops)?;
        Ok(())
    }

    fn question_for(&self, stage: StageId) -> Result<Option<Question>> {
        let row = self
            .conn
            .query_row(
                "SELECT question_id, prompt, selection_type, options_json
                 FROM questions WHERE stage = ?1 AND status = 'pending'",
                params![stage.as_str()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(oops)?;

        let Some((question_id, prompt, selection_type, options_json)) = row else {
            return Ok(None);
        };
        let options: Vec<AnswerOption> =
            serde_json::from_str(&options_json).map_err(|e| StudioError::StateDrift {
                detail: format!("确认门 {question_id} 的选项无法解析：{e}"),
            })?;
        Ok(Some(Question {
            question_id,
            stage,
            prompt,
            selection_type: if selection_type == "multi" {
                SelectionType::Multi
            } else {
                SelectionType::Single
            },
            options,
        }))
    }

    /// 全库唯一的挂起问题（一个 bundle 同一时刻最多一个门）。
    pub fn pending_question(&self) -> Result<Option<Question>> {
        let stage: Option<String> = self
            .conn
            .query_row(
                "SELECT stage FROM questions WHERE status = 'pending' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(oops)?;
        match stage {
            None => Ok(None),
            Some(s) => {
                let stage = StageId::parse(&s).ok_or_else(|| StudioError::StateDrift {
                    detail: format!("确认门挂在未知阶段 {s} 上"),
                })?;
                self.question_for(stage)
            }
        }
    }

    // ---------- events ----------

    pub fn append_event(
        &self,
        stage: StageId,
        kind: &str,
        summary: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO events (at, stage, kind, summary, error) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![now(), stage.as_str(), kind, summary, error],
            )
            .map_err(oops)?;
        Ok(())
    }

    /// 按时间正序返回最近 `limit` 条。
    pub fn timeline(&self, limit: usize) -> Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT at, stage, kind, summary, error FROM events
                 ORDER BY id DESC LIMIT ?1",
            )
            .map_err(oops)?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(oops)?;
        let mut out = Vec::new();
        for row in rows {
            let (at, stage, kind, summary, error) = row.map_err(oops)?;
            out.push(Event {
                at,
                stage: StageId::parse(&stage).unwrap_or(StageId::Idea),
                kind,
                summary,
                error,
            });
        }
        out.reverse();
        Ok(out)
    }

    // ---------- artifacts ----------

    pub fn register_artifact(
        &self,
        stage: StageId,
        kind: &str,
        rel_path: &str,
        meta: Option<&serde_json::Value>,
    ) -> Result<()> {
        debug_assert!(!rel_path.starts_with('/'), "bundle 内一律相对路径");
        let meta_json = match meta {
            Some(v) => Some(serde_json::to_string(v).map_err(oops)?),
            None => None,
        };
        self.conn
            .execute(
                "INSERT INTO artifacts (stage, kind, rel_path, meta_json, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![stage.as_str(), kind, rel_path, meta_json, now()],
            )
            .map_err(oops)?;
        Ok(())
    }

    pub fn artifacts(&self, stage: Option<StageId>) -> Result<Vec<(StageId, String, String)>> {
        let mut out = Vec::new();
        match stage {
            Some(s) => {
                let mut stmt = self
                    .conn
                    .prepare(
                        "SELECT stage, kind, rel_path FROM artifacts WHERE stage = ?1 ORDER BY id",
                    )
                    .map_err(oops)?;
                let rows = stmt
                    .query_map(params![s.as_str()], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(oops)?;
                for row in rows {
                    let (st, kind, path) = row.map_err(oops)?;
                    out.push((StageId::parse(&st).unwrap_or(StageId::Idea), kind, path));
                }
            }
            None => {
                let mut stmt = self
                    .conn
                    .prepare("SELECT stage, kind, rel_path FROM artifacts ORDER BY id")
                    .map_err(oops)?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(oops)?;
                for row in rows {
                    let (st, kind, path) = row.map_err(oops)?;
                    out.push((StageId::parse(&st).unwrap_or(StageId::Idea), kind, path));
                }
            }
        }
        Ok(out)
    }

    // ---------- 撤销槽 ----------

    /// 最多保留多少层撤销。再深就把最旧的丢掉——快照带着完整产物，不能无限长。
    const UNDO_DEPTH: usize = 50;

    /// 压入一份快照。每个改变状态的操作在动手前调用。
    pub fn take_snapshot(&self, label: &str) -> Result<()> {
        let stages = self.dump_table(
            "SELECT stage, state, CAST(attempt AS TEXT), outputs_json, summary FROM stages ORDER BY stage",
            5,
        )?;
        let questions = self.dump_table(
            "SELECT question_id, stage, prompt, selection_type, options_json, status, answer, created_at, answered_at
             FROM questions ORDER BY question_id",
            9,
        )?;
        self.conn
            .execute(
                "INSERT INTO undo_stack (label, taken_at, stages_json, questions_json) VALUES (?1, ?2, ?3, ?4)",
                params![label, now(), stages.to_string(), questions.to_string()],
            )
            .map_err(oops)?;
        self.conn
            .execute(
                "DELETE FROM undo_stack WHERE seq <= (
                     SELECT MAX(seq) FROM undo_stack
                 ) - ?1",
                params![Self::UNDO_DEPTH as i64],
            )
            .map_err(oops)?;
        Ok(())
    }

    /// 栈里还剩几层可撤销。
    pub fn undo_depth(&self) -> Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM undo_stack", [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n as usize)
            .map_err(oops)
    }

    /// 栈顶那份快照的说明文字，栈空则为 None。
    pub fn snapshot_label(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT label FROM undo_stack ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(oops)
    }

    /// 弹出栈顶并恢复。连着调就一步步往回走。
    pub fn restore_snapshot(&self) -> Result<String> {
        let row: Option<(i64, String, String, String)> = self
            .conn
            .query_row(
                "SELECT seq, label, stages_json, questions_json FROM undo_stack ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(oops)?;
        let Some((seq, label, stages_json, questions_json)) = row else {
            return Err(StudioError::InvalidTransition {
                stage: StageId::Idea,
                current: "no_snapshot",
                attempted: "undo",
                allowed: vec!["studio.revise", "studio.status"],
            });
        };

        let stages: Vec<Vec<Option<String>>> = serde_json::from_str(&stages_json).map_err(oops)?;
        let questions: Vec<Vec<Option<String>>> =
            serde_json::from_str(&questions_json).map_err(oops)?;

        self.conn
            .execute("DELETE FROM questions", [])
            .map_err(oops)?;
        for q in &questions {
            self.conn
                .execute(
                    "INSERT INTO questions
                       (question_id, stage, prompt, selection_type, options_json, status, answer, created_at, answered_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![q[0], q[1], q[2], q[3], q[4], q[5], q[6], q[7], q[8]],
                )
                .map_err(oops)?;
        }
        for st in &stages {
            self.conn
                .execute(
                    "UPDATE stages SET state = ?2, attempt = ?3, outputs_json = ?4, summary = ?5, updated_at = ?6
                     WHERE stage = ?1",
                    params![st[0], st[1], st[2], st[3], st[4], now()],
                )
                .map_err(oops)?;
        }
        self.conn
            .execute("DELETE FROM undo_stack WHERE seq = ?1", params![seq])
            .map_err(oops)?;
        self.reseal()?;
        Ok(label)
    }

    fn dump_table(&self, sql: &str, cols: usize) -> Result<serde_json::Value> {
        let mut stmt = self.conn.prepare(sql).map_err(oops)?;
        let rows = stmt
            .query_map([], |r| {
                let mut row = Vec::with_capacity(cols);
                for i in 0..cols {
                    row.push(r.get::<_, Option<String>>(i)?);
                }
                Ok(row)
            })
            .map_err(oops)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(oops)?);
        }
        serde_json::to_value(out).map_err(oops)
    }

    // ---------- 完整性 ----------

    /// 覆盖 stages 与 questions 的摘要。事件与产物不参与——它们是追加日志，
    /// 追加不构成「状态被篡改」。
    fn digest(&self) -> Result<String> {
        let mut h = Sha256::new();
        let mut stmt = self
            .conn
            .prepare("SELECT stage, state, attempt, COALESCE(outputs_json,'') FROM stages ORDER BY stage")
            .map_err(oops)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}|{}|{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?
                ))
            })
            .map_err(oops)?;
        for row in rows {
            h.update(row.map_err(oops)?.as_bytes());
            h.update(b"\n");
        }
        h.update(b"--questions--\n");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT question_id, stage, prompt, selection_type, options_json, status, COALESCE(answer,'')
                 FROM questions ORDER BY question_id",
            )
            .map_err(oops)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?
                ))
            })
            .map_err(oops)?;
        for row in rows {
            h.update(row.map_err(oops)?.as_bytes());
            h.update(b"\n");
        }
        Ok(format!("{:x}", h.finalize()))
    }

    fn reseal(&self) -> Result<()> {
        let d = self.digest()?;
        self.conn
            .execute("UPDATE integrity SET digest = ?1 WHERE id = 1", params![d])
            .map_err(oops)?;
        Ok(())
    }

    /// 外部改动过 `.studio/` 就会在这里暴露。
    pub fn verify_integrity(&self) -> Result<()> {
        let stored: Option<String> = self
            .conn
            .query_row("SELECT digest FROM integrity WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(oops)?;
        let Some(stored) = stored else {
            return Err(StudioError::StateDrift {
                detail: "完整性记录缺失".into(),
            });
        };
        let actual = self.digest()?;
        if stored != actual {
            return Err(StudioError::StateDrift {
                detail: "stages / questions 的内容与上次写入时的摘要不一致".into(),
            });
        }
        Ok(())
    }

    /// 用户确认后把摘要重新盖章。仅供 `studiod doctor --repair` 使用。
    pub fn reseal_after_manual_repair(&self) -> Result<()> {
        self.reseal()
    }
}

fn parse_outputs(s: &str, stage: StageId) -> Result<Outputs> {
    serde_json::from_str(s).map_err(|e| StudioError::StateDrift {
        detail: format!("阶段 {stage} 的产物无法解析：{e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::contract::AnswerOption;
    use studio_core::state::{Draft, Stage};
    use studio_core::Confirmation;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::create(
            &dir.path().join("studio.db"),
            "千岛湖，把快乐装进十秒",
            "0.1.0",
        )
        .unwrap();
        (dir, s)
    }

    fn outs(key: &str) -> Outputs {
        let mut m = Outputs::new();
        m.insert(key.into(), serde_json::json!({ "ok": true }));
        m
    }

    fn conf() -> Confirmation {
        Confirmation {
            prompt: "是否确认这版剧本？".into(),
            selection_type: SelectionType::Single,
            options: vec![
                AnswerOption::new("approve", "确认"),
                AnswerOption::new("revise", "修改"),
            ],
        }
    }

    #[test]
    fn fresh_project_starts_with_every_stage_in_draft() {
        let (_d, s) = store();
        assert_eq!(s.title().unwrap(), "千岛湖，把快乐装进十秒");
        for stage in StageId::all() {
            assert_eq!(s.load_stage(stage).unwrap().state(), StageState::Draft);
        }
        assert!(s.pending_question().unwrap().is_none());
    }

    #[test]
    fn gate_roundtrips_through_the_database() {
        let (_d, s) = store();
        let submitted = Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(conf()))
            .unwrap();
        let q = submitted.question().unwrap().clone();
        s.save_stage(
            StageId::Script,
            StageState::AwaitingConfirmation,
            1,
            submitted.outputs(),
            Some("已完成剧本"),
            Some(&q),
        )
        .unwrap();

        let loaded = s.load_stage(StageId::Script).unwrap();
        assert_eq!(loaded.state(), StageState::AwaitingConfirmation);
        let pending = s.pending_question().unwrap().unwrap();
        assert_eq!(pending.question_id, "script.approval");
        assert!(pending.accepts("approve"));
        assert_eq!(
            s.stage_summary(StageId::Script).unwrap().as_deref(),
            Some("已完成剧本")
        );
    }

    /// 门在离开挂起状态时整行删除，不留「已取消」的残骸——
    /// 前身项目正是因为取消记录还在，重新提交时门挂不回去。
    #[test]
    fn revise_clears_the_gate_completely() {
        let (_d, s) = store();
        let submitted = Stage::<Draft>::new(StageId::Script)
            .submit(outs("script"), Some(conf()))
            .unwrap();
        let q = submitted.question().unwrap().clone();
        s.save_stage(
            StageId::Script,
            StageState::AwaitingConfirmation,
            1,
            submitted.outputs(),
            None,
            Some(&q),
        )
        .unwrap();
        assert!(s.pending_question().unwrap().is_some());

        // 用户要求修订
        s.save_stage(
            StageId::Script,
            StageState::Draft,
            2,
            Some(&outs("script")),
            None,
            None,
        )
        .unwrap();
        assert!(
            s.pending_question().unwrap().is_none(),
            "修订后不应残留任何挂起的门"
        );

        // 立刻重新提交，同一个 question_id 必须能重新挂起
        let again = Stage::<Draft>::resumed(StageId::Script, 2, None)
            .submit(outs("script"), Some(conf()))
            .unwrap();
        let q2 = again.question().unwrap().clone();
        s.save_stage(
            StageId::Script,
            StageState::AwaitingConfirmation,
            2,
            again.outputs(),
            None,
            Some(&q2),
        )
        .unwrap();
        let pending = s.pending_question().unwrap().unwrap();
        assert_eq!(pending.question_id, "script.approval");
        assert_eq!(s.load_stage(StageId::Script).unwrap().attempt(), 2);
    }

    #[test]
    fn timeline_is_chronological() {
        let (_d, s) = store();
        s.append_event(StageId::Idea, "submitted", "brief 完成", None)
            .unwrap();
        s.append_event(StageId::Selection, "gate_opened", "等待确认", None)
            .unwrap();
        s.append_event(StageId::Selection, "answered", "已确认", None)
            .unwrap();
        let t = s.timeline(10).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].stage, StageId::Idea);
        assert_eq!(t[2].kind, "answered");
    }

    #[test]
    fn timeline_limit_keeps_the_latest() {
        let (_d, s) = store();
        for i in 0..10 {
            s.append_event(StageId::Idea, "tick", &format!("第 {i} 条"), None)
                .unwrap();
        }
        let t = s.timeline(3).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[2].summary, "第 9 条");
    }

    #[test]
    fn artifacts_are_registered_per_stage() {
        let (_d, s) = store();
        s.register_artifact(StageId::Render, "video", "media/sh01.mp4", None)
            .unwrap();
        s.register_artifact(StageId::Post, "video", "output/final.mp4", None)
            .unwrap();
        assert_eq!(s.artifacts(None).unwrap().len(), 2);
        assert_eq!(
            s.artifacts(Some(StageId::Post)).unwrap()[0].2,
            "output/final.mp4"
        );
    }

    /// 直接改库会被下一次打开时抓到——这是 .studio/ 私有约定的兜底。
    #[test]
    fn external_tampering_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("studio.db");
        {
            let s = Store::create(&path, "t", "0.1.0").unwrap();
            s.save_stage(
                StageId::Idea,
                StageState::Approved,
                1,
                Some(&outs("brief")),
                None,
                None,
            )
            .unwrap();
        }
        // 绕过控制面直接改状态，正是前身项目那次做的事
        {
            let c = Connection::open(&path).unwrap();
            c.execute("UPDATE stages SET state = 'draft' WHERE stage = 'idea'", [])
                .unwrap();
        }
        let e = Store::open(&path).unwrap_err();
        assert_eq!(e.code(), "state_drift");
        assert!(e.remedy().contains("studio."));
    }

    #[test]
    fn reopening_an_untouched_project_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("studio.db");
        {
            let s = Store::create(&path, "t", "0.1.0").unwrap();
            s.save_stage(
                StageId::Idea,
                StageState::Approved,
                1,
                Some(&outs("brief")),
                None,
                None,
            )
            .unwrap();
            s.append_event(StageId::Idea, "submitted", "ok", None)
                .unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(
            s.load_stage(StageId::Idea).unwrap().state(),
            StageState::Approved
        );
    }
}
