//! Form behavior shared by the concrete settings pages.
use super::*;

impl SettingsUi {
    pub(super) fn draw_controls_menu(&mut self, f: &mut Frame) {
        self.hits.clear();
        let values = self.controls();
        let selected = self.control_menu.unwrap_or(0);
        let r = dialog(self.area, 72, (values.len() + 4) as u16);
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(tr("操作 · Esc 返回", "Actions · Esc returns"))
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        let first = selected.saturating_sub(inner.height.saturating_sub(1) as usize);
        for (i, (label, _)) in values
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height as usize)
        {
            let row = Rect::new(inner.x, inner.y + (i - first) as u16, inner.width, 1);
            let enabled = self.hit_enabled(&values[i].1);
            let p = crate::appearance::palette();
            f.render_widget(
                Paragraph::new(format!("{} {label}", if i == selected { "▸" } else { " " })).style(
                    (if i == selected {
                        Style::new().bg(crate::appearance::palette().selected)
                    } else {
                        Style::default()
                    })
                    .fg(if !enabled {
                        p.muted
                    } else if Self::hit_dangerous(&values[i].1) {
                        p.error
                    } else {
                        p.fg
                    }),
                ),
                row,
            );
            self.hits.push((row, Hit::Control(i)));
        }
    }

    pub(super) fn pick_project(&mut self) -> Result<()> {
        let (path, field) =
            if self.page().field.key == "excluded" && self.children(&self.page()).is_empty() {
                let page = self.page();
                let Kind::List { item, .. } = page.field.kind else {
                    unreachable!()
                };
                (draft::index(&page.path, 0), *item)
            } else {
                self.row().context("Select a project path first")?
            };
        anyhow::ensure!(
            matches!(field.kind, Kind::Text { path: true, .. }),
            "Select a project path first"
        );
        anyhow::ensure!(
            !field.readonly || self.value(&path, &field).as_str() == Some(""),
            "Existing project identity is read-only"
        );
        let snapshot = Snapshot::parse(toml::to_string(
            &self
                .draft
                .as_ref()
                .context("Repair configuration first")?
                .tree,
        )?)?;
        snapshot.require("discovery")?;
        self.project_picker = Some((path, field, vec![], 0));
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let mut values =
                crate::settings::projection::from_preferences(&snapshot.effective).scan();
            values.extend(snapshot.effective.project.iter().map(|p| p.path.clone()));
            values.extend(snapshot.effective.discovery.added);
            values.sort_by_key(|p| crate::config::key(p));
            values.dedup_by_key(|p| crate::config::key(p));
            let _ = tx.send(Reply::Projects(Ok(values
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect())));
        });
        Ok(())
    }
    pub(super) fn draw_project_picker(&mut self, f: &mut Frame) {
        self.hits.clear();
        let r = dialog(self.area, 100, self.area.height.saturating_sub(2));
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(tr(
                    "选择已知项目 · Esc 取消",
                    "Choose a known project · Esc cancels",
                ))
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        let (_, _, values, selected) = self.project_picker.as_ref().unwrap();
        let count = inner.height.saturating_sub(1) as usize;
        let first = selected.saturating_sub(count.saturating_sub(1));
        for (i, value) in values.iter().enumerate().skip(first).take(count) {
            let row = Rect::new(inner.x, inner.y + (i - first) as u16, inner.width, 1);
            f.render_widget(
                Paragraph::new(format!(
                    "{} {value}",
                    if i == *selected { "▸" } else { " " }
                ))
                .style(if i == *selected {
                    Style::new().bg(crate::appearance::palette().selected)
                } else {
                    Style::default()
                }),
                row,
            );
            self.hits.push((row, Hit::Picked(i)));
        }
        if values.is_empty() {
            f.render_widget(
                Paragraph::new(tr(
                    "暂无候选；可取消后直接输入绝对路径。",
                    "No candidates; cancel and enter an absolute path directly.",
                )),
                inner,
            );
        }
        self.button(
            f,
            Rect::new(
                inner.x,
                inner.bottom().saturating_sub(1),
                inner.width.min(20),
                1,
            ),
            tr("取消 Esc", "Cancel Esc"),
            Hit::Cancel,
        );
    }

    pub(super) fn contextual_help(&self, path: &[Part], field: &Field, value: &Value) -> String {
        let agent = self
            .draft
            .as_ref()
            .and_then(|d| {
                if path.len() >= 4 {
                    draft::get(&d.tree, &draft::key(&path[..path.len() - 1], "agent"))
                } else {
                    d.tree.get("launch").and_then(|v| v.get("agent"))
                }
            })
            .and_then(Value::as_str);
        match field.key {
            "layout" => tr(
                "浮层 [Agent + Debug□] · 分屏 [Agent│Debug] · 标签页 [Agent][Debug]",
                "Floating [Agent + Debug□] · Split [Agent│Debug] · Tabs [Agent][Debug]",
            ),
            "compact" => tr(
                "示意：src → settings → ui.rs  ⇄  src/settings → ui.rs",
                "Example: src → settings → ui.rs  ⇄  src/settings → ui.rs",
            ),
            "wrap" => tr(
                "示意：长行继续… ↩ 下一显示行；仅新文档采用。",
                "Example: a long line… ↩ next display row. New documents only.",
            ),
            "width" => tr(
                "[文件树│终端] 20–80 列；仅新视图采用。",
                "[Tree│Terminal] 20–80 columns; new views only.",
            ),
            "history_lines" => format!(
                "{} · {}",
                field.help,
                if value.as_integer() == Some(0) {
                    tr("不保留滚动历史", "Scrollback disabled")
                } else {
                    tr("单位：行", "Unit: lines")
                }
            ),
            "timeout_ms" => format!(
                "{} ms = {:.3} s · {}",
                value.as_integer().unwrap_or(0),
                value.as_integer().unwrap_or(0) as f64 / 1000.0,
                tr(
                    "1–3600000 毫秒；仅编辑，不执行",
                    "1–3600000 milliseconds; editing never executes"
                )
            ),
            "resume_latest" | "agent_args" if agent == Some("shell") => tr(
                "Shell 下不适用；保留值以便以后切换 Agent。",
                "Not used by Shell; values are retained for a future Agent.",
            ),
            "debug_command" => tr(
                "空值只打开 PowerShell；仅 Agent 组合不使用此命令。",
                "Empty opens PowerShell only; unused in Agent-only composition.",
            ),
            "sessions" => tr(
                "仅自定义组合启用；保留当前条目，添加不会自动切换组合。",
                "Enabled by Custom composition; adding entries never changes the composition.",
            ),
            "agent_ratio" => tr(
                "[Agent│预览] 0.01–99.99%；E 精确输入，不影响 Debug。",
                "[Agent│Preview] 0.01–99.99%; E for exact input. Debug is unaffected.",
            ),
            _ => field.help.into(),
        }
    }

    pub(super) fn decide_path(&mut self, which: u8) -> Result<()> {
        if which == 2 {
            self.path_decision = None;
            return Ok(());
        }
        let (path, value) = self
            .path_decision
            .take()
            .context("No path change pending")?;
        self.change(&path, Some(value))?;
        if which == 1 {
            self.recapture()?;
        }
        Ok(())
    }
    pub(super) fn draw_path_decision(&mut self, f: &mut Frame) {
        self.hits.clear();
        let r = dialog(self.area, 80, 11);
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(tr("修改启动项目", "Change launch project"))
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        let (_, value) = self.path_decision.as_ref().unwrap();
        f.render_widget(Paragraph::new(format!("{}\n{}",value,tr("保留当前快照，或从新项目重新捕获全部启动值。取消保持原路径及快照。","Keep the current snapshot, or recapture all launch values from the new project. Cancel retains the original path and snapshot."))).wrap(Wrap{trim:false}),Rect::new(inner.x,inner.y,inner.width,inner.height.saturating_sub(3)));
        self.buttons(
            f,
            Rect::new(inner.x, inner.bottom().saturating_sub(3), inner.width, 3),
            vec![
                (tr("取消 Esc", "Cancel Esc"), Hit::PathDecision(2)),
                (tr("保留快照 K", "Keep snapshot K"), Hit::PathDecision(0)),
                (tr("重新捕获 R", "Recapture R"), Hit::PathDecision(1)),
            ],
        );
    }
    pub(super) fn fine_step(&mut self, delta: i64) -> Result<()> {
        if let Some((path, field)) = self.row()
            && let Kind::Number {
                min, max, percent, ..
            } = field.kind
        {
            if self.inherited_lock(&path, &field) {
                return Ok(());
            }
            let value = self.value(&path, &field).as_integer().unwrap_or(min);
            self.change(
                &path,
                Some(
                    (value + delta * if percent { 100 } else { 1 })
                        .clamp(min, max)
                        .into(),
                ),
            )?;
        }
        Ok(())
    }
    pub(super) fn controls(&self) -> Vec<(String, Hit)> {
        let mut controls = vec![
            (tr("返回 ←", "Back ←"), Hit::Back),
            (tr("详情 F1", "Details F1"), Hit::FieldHelp),
        ];
        if self.stack.len() > 1 {
            controls.push((tr("取消子编辑 Esc", "Cancel child Esc"), Hit::Cancel));
        }
        let page = self.page();
        if let Kind::List { ordered, .. } = page.field.kind {
            controls.push((tr("添加 Insert", "Add Insert"), Hit::Add));
            if !self.children(&page).is_empty() {
                controls.push((tr("删除 Delete", "Delete"), Hit::Delete));
            }
            if ordered {
                if self.selected() > 0 {
                    controls.push((tr("上移 Alt+↑", "Move Alt+↑"), Hit::Up));
                }
                if self.selected() + 1 < self.children(&page).len() {
                    controls.push((tr("下移 Alt+↓", "Move Alt+↓"), Hit::Down));
                }
            }
        } else if matches!(page.field.kind, Kind::Map) {
            controls.extend([
                (tr("添加 Insert", "Add Insert"), Hit::Add),
                (tr("删除 Delete", "Delete"), Hit::Delete),
            ]);
        }
        if let Some((path, field)) = self.row() {
            if self.inherited_lock(&path, &field) {
                controls.push((tr("转为覆盖 O", "Override O"), Hit::Override));
            }
            if field.optional {
                controls.push((tr("恢复默认/继承 F2", "Default / inherit F2"), Hit::Inherit));
            }
            if matches!(field.kind, Kind::Number { .. }) && !self.inherited_lock(&path, &field) {
                controls.extend([("−".into(), Hit::Step(-1)), ("+".into(), Hit::Step(1))]);
            }
            if matches!(field.kind, Kind::Text { path: true, .. })
                || matches!(field.key, "agent" | "shell")
            {
                controls.push((tr("探测 Alt+P", "Probe Alt+P"), Hit::Probe));
            }
            if matches!(field.kind, Kind::Text { .. }) || matches!(field.key, "shell" | "color") {
                controls.push((tr("复制 Alt+C", "Copy Alt+C"), Hit::Copy));
            }
        }
        if self.page().field.key == "excluded"
            || self.row().is_some_and(|(_, f)| {
                matches!(f.kind, Kind::Text { path: true, .. })
                    && (!f.readonly
                        || self
                            .row()
                            .is_some_and(|(p, _)| self.value(&p, &f).as_str() == Some("")))
            })
        {
            controls.push((
                tr("选择项目 Alt+L", "Choose project Alt+L"),
                Hit::PickProject,
            ));
        }
        if self.entry_path().is_some() {
            controls.push((tr("重新捕获 F7", "Recapture F7"), Hit::Recapture));
        }
        if self.can_queue && self.category == 9 && self.page().path.len() >= 2 {
            controls.push((tr("加入队列 F9", "Queue F9"), Hit::ApplyPreset));
        }
        controls
    }
    pub(super) fn restore_toggle(&mut self, i: usize) {
        if let Some(plan) = &mut self.restore
            && let Some((id, _, _)) = plan.conflicts.get(i)
        {
            if !plan.use_archive.remove(id) {
                plan.use_archive.insert(id.clone());
            }
        }
    }
    pub(super) fn restore_key(&mut self, key: KeyEvent) -> Result<Outcome> {
        if self.pending.is_some() {
            return Ok(Outcome::Stay);
        }
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => self.restore = None,
            (KeyCode::PageUp, _) => self.restore_scroll = self.restore_scroll.saturating_sub(8),
            (KeyCode::PageDown, _) => self.restore_scroll = self.restore_scroll.saturating_add(8),
            (KeyCode::Up, _) => self.restore_selected = self.restore_selected.saturating_sub(1),
            (KeyCode::Down, _) => {
                self.restore_selected = (self.restore_selected + 1).min(
                    self.restore
                        .as_ref()
                        .unwrap()
                        .conflicts
                        .len()
                        .saturating_sub(1),
                )
            }
            (KeyCode::Char(' '), _) => self.restore_toggle(self.restore_selected),
            (KeyCode::Enter, Mods::CONTROL) => self.restore_apply()?,
            _ => {}
        }
        Ok(Outcome::Stay)
    }
    pub(super) fn restore_apply(&mut self) -> Result<()> {
        if self.pending.is_some() {
            return Ok(());
        }
        let plan = self
            .restore
            .as_ref()
            .context("Inspect archive before restoring")?
            .clone();
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(Reply::Operation(
                super::super::operations::commit_restore(&root, &plan)
                    .map_err(|e| format!("{e:#}")),
            ));
        });
        self.restore = None;
        Ok(())
    }
    pub(super) fn draw_restore(&mut self, f: &mut Frame) {
        self.hits.clear();
        let plan = self.restore.as_ref().unwrap();
        let r = dialog(self.area, 100, self.area.height.saturating_sub(2));
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(tr(
                    "恢复预览 · 冲突默认保留当前值",
                    "Restore preview · conflicts keep current values by default",
                ))
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        let mut text = vec![
            plan.summary(),
            tr(
                "↑↓ 选择冲突 · Space 切换采用归档 · Ctrl+Enter 确认恢复 · Esc 取消",
                "↑↓ select conflict · Space use archive / keep current · Ctrl+Enter confirm restore · Esc cancel",
            ),
        ];
        if let Some((id, current, archive)) = plan.conflicts.get(self.restore_selected) {
            text.extend([
                format!(
                    "{}/{} {} [{}]",
                    self.restore_selected + 1,
                    plan.conflicts.len(),
                    id,
                    if plan.use_archive.contains(id) {
                        "use archive / 采用归档"
                    } else {
                        "keep current / 保留当前"
                    }
                ),
                format!("Current / 当前:\n{current}"),
                format!("Archive / 归档:\n{archive}"),
            ]);
        } else {
            text.push(tr("新增条目将合并，同值保持；不会删除当前独有内容。","New entries are merged and identical entries retained; current-only content remains."));
        }
        f.render_widget(
            Paragraph::new(text.join("\n\n"))
                .wrap(Wrap { trim: false })
                .scroll((self.restore_scroll, 0)),
            Rect::new(
                inner.x,
                inner.y,
                inner.width,
                inner.height.saturating_sub(3),
            ),
        );
        self.buttons(
            f,
            Rect::new(inner.x, inner.bottom().saturating_sub(3), inner.width, 3),
            vec![
                (tr("取消", "Cancel"), Hit::RestoreCancel),
                (
                    tr("切换当前冲突", "Toggle selected conflict"),
                    Hit::RestoreToggle(self.restore_selected),
                ),
                (
                    tr("确认合并 Ctrl+Enter", "Confirm merge Ctrl+Enter"),
                    Hit::RestoreApply,
                ),
            ],
        );
    }
    pub(super) fn inherited_lock(&self, path: &[Part], field: &Field) -> bool {
        if !matches!(path.first(),Some(Part::Key(k)) if k=="project") || path.len() < 4 {
            return false;
        }
        if !matches!(&path[2],Part::Key(k) if matches!(k.as_str(),"launch"|"sidebar")) {
            return false;
        }
        // An inherited array is a single override, not independently editable children.
        let leaf = &path[..4];
        self.draft
            .as_ref()
            .is_some_and(|d| draft::get(&d.tree, leaf).is_none())
            && (field.optional || path.len() > 4)
    }
    pub(super) fn choose(&mut self, i: usize) -> Result<()> {
        let Some(choice) = self.choice.take() else {
            return Ok(());
        };
        if let Kind::Choice(ref values) = choice.field.kind
            && let Some(v) = values.get(i)
        {
            if *v == "$custom" {
                let value = self.value(&choice.path, &choice.field);
                self.start_edit(choice.path, choice.field, value, None);
                return Ok(());
            }
            let value = if matches!(*v, "true" | "false") {
                (*v == "true").into()
            } else {
                (*v).into()
            };
            self.change(&choice.path, Some(value))?;
        }
        Ok(())
    }
    pub(super) fn draw_choice(&mut self, f: &mut Frame) {
        self.hits.clear();
        let choice = self.choice.as_ref().unwrap();
        let Kind::Choice(values) = &choice.field.kind else {
            return;
        };
        let values = values.clone();
        let selected = choice.selected;
        let field = choice.field.clone();
        let r = dialog(self.area, 72, (values.len() + 5) as u16);
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(field.label())
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        let available = inner.height.saturating_sub(2) as usize;
        let first = selected.saturating_sub(available.saturating_sub(1));
        for (i, v) in values.iter().enumerate().skip(first).take(available) {
            let row = Rect::new(inner.x, inner.y + (i - first) as u16, inner.width, 1);
            f.render_widget(
                Paragraph::new(format!(
                    "{} {}",
                    if selected == i { "●" } else { "○" },
                    option_label(field.key, v)
                ))
                .style(if selected == i {
                    Style::new().bg(crate::appearance::palette().selected)
                } else {
                    Style::default()
                }),
                row,
            );
            if field.key == "color"
                && let Some(color) = parse_color(v)
            {
                f.render_widget(
                    Paragraph::new("■■").style(Style::new().fg(color)),
                    Rect::new(row.right().saturating_sub(3), row.y, 2.min(row.width), 1),
                );
            }
            if field.key == "theme" {
                let theme = match *v {
                    "tokyo" => crate::appearance::Theme::Tokyo,
                    "catppuccin" => crate::appearance::Theme::Catppuccin,
                    "light" => crate::appearance::Theme::Light,
                    _ => crate::appearance::Theme::Terminal,
                };
                let palette = crate::appearance::palette_for(theme);
                if row.width > 12 {
                    for (j, color) in [palette.bg, palette.fg, palette.accent, palette.selected]
                        .iter()
                        .enumerate()
                    {
                        f.render_widget(
                            Paragraph::new("■■").style(Style::new().fg(*color)),
                            Rect::new(row.right() - 10 + j as u16 * 2, row.y, 2, 1),
                        );
                    }
                }
            }
            self.hits.push((row, Hit::Choice(i)));
        }
        f.render_widget(
            Paragraph::new(tr(
                "↑↓ 浏览 · Enter 确认 · Esc 取消",
                "↑↓ browse · Enter confirm · Esc cancel",
            )),
            Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
        );
    }
    pub(super) fn issues(&self) -> Vec<(Path, String)> {
        let Some(d) = &self.draft else {
            return vec![];
        };
        let Ok(snapshot) = toml::to_string(&d.tree)
            .ok()
            .and_then(|s| Snapshot::parse(s).ok())
            .ok_or(())
        else {
            return vec![];
        };
        let mut paths = vec![];
        fn collect(tree: &Value, path: Path, name: String, out: &mut Vec<(Path, String)>) {
            out.push((path.clone(), name.clone()));
            match tree {
                Value::Table(t) => {
                    for (k, v) in t {
                        collect(
                            v,
                            draft::key(&path, k),
                            if name.is_empty() {
                                k.clone()
                            } else {
                                format!("{name}.{k}")
                            },
                            out,
                        );
                    }
                }
                Value::Array(a) => {
                    for (i, v) in a.iter().enumerate() {
                        let identity = if name == "project" {
                            v.get("path")
                                .and_then(Value::as_str)
                                .map(|s| super::super::model::project_key(std::path::Path::new(s)))
                        } else if name == "preset" {
                            v.get("name").and_then(Value::as_str).map(str::to_owned)
                        } else {
                            None
                        };
                        collect(
                            v,
                            draft::index(&path, i),
                            format!("{name}[{}]", identity.unwrap_or_else(|| i.to_string())),
                            out,
                        );
                    }
                }
                _ => {}
            }
        }
        collect(&d.tree, vec![], String::new(), &mut paths);
        snapshot.diagnostics.into_iter().map(|issue|{
            let mut candidates=paths.iter().filter(|(_,p)|*p==issue.path||issue.path.starts_with(&format!("{p}."))||issue.path.starts_with(&format!("{p}["))).collect::<Vec<_>>();
            candidates.sort_by_key(|(_,p)|p.len());
            let mut path=candidates.last().map_or_else(||draft::key(&[],"data"),|(p,_)|p.clone());
            // Nested deserialization errors often name the leaf in their message.
            if let Some((leaf,_))=paths.iter().filter(|(p,_)|p.starts_with(&path)).find(|(p,_)|matches!(p.last(),Some(Part::Key(k)) if issue.message.contains(&format!("`{k}`")))) {path=leaf.clone();}
            (path,format!("{}: {}",issue.path,issue.message))
        }).collect()
    }
    pub(super) fn draw_issues(&mut self, f: &mut Frame) {
        self.hits.clear();
        let issues = self.issues();
        let r = dialog(self.area, 100, self.area.height.saturating_sub(2));
        crate::ui::clear_popup(f, r);
        f.render_widget(
            Block::bordered()
                .title(tr(
                    "配置诊断 · Enter 定位 · Esc 返回",
                    "Configuration diagnostics · Enter locate · Esc back",
                ))
                .style(crate::appearance::style()),
            r,
        );
        let inner = r.inner(ratatui::layout::Margin::new(1, 1));
        if issues.is_empty() {
            f.render_widget(
                Paragraph::new(tr("当前没有配置诊断。", "No configuration diagnostics.")),
                inner,
            );
            return;
        }
        let slots = (inner.height / 3).max(1) as usize;
        let first = self.issue_selected.saturating_sub(slots - 1);
        for (i, (_, msg)) in issues.iter().enumerate().skip(first).take(slots) {
            let row = Rect::new(
                inner.x,
                inner.y + ((i - first) * 3) as u16,
                inner.width,
                inner.height.min(3),
            );
            f.render_widget(
                Paragraph::new(msg.clone())
                    .wrap(Wrap { trim: false })
                    .style(if i == self.issue_selected {
                        Style::new().bg(crate::appearance::palette().selected)
                    } else {
                        Style::default()
                    }),
                row,
            );
            self.hits.push((row, Hit::Issue(i)));
        }
    }
    pub(super) fn start_probe(&mut self) {
        let Some((path, field)) = self.row() else {
            return;
        };
        if !matches!(field.kind, Kind::Text { path: true, .. })
            && !matches!(field.key, "agent" | "shell")
        {
            return;
        }
        let value = self.value(&path, &field);
        let value = value.as_str().unwrap_or("").to_owned();
        let (tx, rx) = mpsc::channel();
        self.probe = Some(rx);
        self.message = tr("正在检查路径…", "Checking path…");
        std::thread::spawn(move || {
            let result = if value.is_empty() {
                "Empty: uses the documented working-directory default".into()
            } else {
                let p = std::path::Path::new(&value);
                if p.is_absolute() {
                    match std::fs::metadata(p) {
                        Ok(m) => {
                            if m.is_dir() {
                                "Directory exists".into()
                            } else {
                                "File exists (not a directory)".into()
                            }
                        }
                        Err(e) => format!("Unavailable; may still be saved: {e}"),
                    }
                } else if matches!(field.key, "agent" | "shell") {
                    let resolved = std::env::var_os("PATH").and_then(|p| {
                        std::env::split_paths(&p)
                            .flat_map(|p| {
                                [
                                    p.join(&value),
                                    p.join(format!("{value}.exe")),
                                    p.join(format!("{value}.cmd")),
                                ]
                            })
                            .find(|p| p.is_file())
                    });
                    resolved.map_or_else(
                        || "Not found on PATH; may still be saved".into(),
                        |p| format!("Found on PATH: {} (not executed)", p.display()),
                    )
                } else {
                    "Relative path: resolved from Task.cwd / checkout at execution; not probed against the settings process".into()
                }
            };
            let _ = tx.send(PathProbe {
                path,
                value,
                result,
            });
        });
    }
    pub(super) fn poll_probe(&mut self) -> bool {
        if let Some(probe) = self.probe.as_ref().and_then(|p| p.try_recv().ok()) {
            self.probe = None;
            if self
                .draft
                .as_ref()
                .and_then(|d| draft::get(&d.tree, &probe.path))
                .and_then(Value::as_str)
                == Some(&probe.value)
            {
                self.message = probe.result.clone();
                self.probes.retain(|p| p.path != probe.path);
                self.probes.push(probe);
                return true;
            }
        }
        false
    }
    pub(super) fn entry_path(&self) -> Option<Path> {
        let p = self.page().path;
        if matches!(p.first(),Some(Part::Key(k)) if k=="preset")
            && p.len() >= 4
            && matches!(&p[2],Part::Key(k) if k=="entries")
        {
            Some(p[..4].to_vec())
        } else {
            None
        }
    }
    pub(super) fn recapture(&mut self) -> Result<()> {
        let path = self
            .entry_path()
            .context("Open a preset launch entry first")?;
        let d = self.draft.as_ref().context("Repair configuration first")?;
        let entry = draft::get(&d.tree, &path).context("Launch entry missing")?;
        let project = entry
            .get("project")
            .and_then(Value::as_str)
            .context("Enter a project path first")?;
        let project = PathBuf::from(project);
        anyhow::ensure!(project.is_absolute(), "Project path must be absolute");
        let source = Snapshot::parse(toml::to_string(&d.tree)?)?;
        source.require("launch")?;
        source.require(&format!(
            "{}.launch",
            super::super::model::project_scope(&project)
        ))?;
        let snapshot = source
            .effective
            .resolve(&project, &Default::default())
            .snapshot;
        self.change(&path, Some(Value::try_from(snapshot)?))?;
        self.message = tr(
            "已从当前全局/项目草稿捕获完整快照，尚未保存。",
            "Captured a complete snapshot from current global/project drafts; not saved.",
        );
        Ok(())
    }
    pub(super) fn apply_preset(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.can_queue,
            "Apply presets from the overview settings centre"
        );

        anyhow::ensure!(
            !self.dirty(),
            "Save or discard configuration drafts before queuing a committed preset"
        );
        let page = self.page();
        let i = match page.path.get(1) {
            Some(Part::Index(i)) => *i,
            _ => {
                anyhow::bail!("Open a committed preset first");
            }
        };
        let d = self.draft.as_ref().context("Repair configuration first")?;
        let name = d
            .base
            .tree
            .get("preset")
            .and_then(|p| p.get(i))
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .context("Preset disappeared")?;
        let preset = d
            .base
            .effective
            .preset
            .iter()
            .find(|p| p.name == name)
            .context("Preset is invalid or no longer exists")?;
        for entry in &preset.entries {
            entry.validate()?;
        }
        self.queued_preset = Some(
            preset
                .entries
                .iter()
                .map(super::super::model::LaunchSnapshot::options)
                .collect(),
        );
        self.message = tr(
            "已加入启动队列；关闭设置后查看。没有启动进程。",
            "Queued. Close settings to inspect the launch queue. No process started.",
        );
        Ok(())
    }
    pub(super) fn draw_control(
        &mut self,
        f: &mut Frame,
        r: Rect,
        path: &Path,
        field: &Field,
        value: &Value,
    ) {
        if r.height == 0 || r.width == 0 || self.inherited_lock(path, field) {
            return;
        }
        if let Kind::Number {
            min,
            max,
            percent: true,
            ..
        } = field.kind
        {
            let n = value.as_integer().unwrap_or(min);
            let w = r.width.saturating_sub(15).min(32);
            if w < 3 {
                return;
            }
            let pos = ((n.clamp(min, max) - min) * (w - 1) as i64 / (max - min)) as u16;
            let text = (0..w)
                .map(|i| {
                    if i == pos {
                        '●'
                    } else if i < pos {
                        '━'
                    } else {
                        '─'
                    }
                })
                .collect::<String>();
            f.render_widget(
                Paragraph::new(format!("{text} {}.{:02}%", n / 100, n % 100))
                    .style(Style::new().fg(crate::appearance::palette().accent)),
                r,
            );
            for i in 0..w {
                let v = min + i64::from(i) * (max - min) / i64::from(w - 1);
                self.hits
                    .push((Rect::new(r.x + i, r.y, 1, 1), Hit::Slider(path.clone(), v)));
            }
        } else if let Kind::Choice(values) = &field.kind {
            use unicode_width::UnicodeWidthStr;
            let total: usize = values
                .iter()
                .map(|v| option_label(field.key, v).width() + 4)
                .sum();
            if field.key == "color" {
                if let Some(color) = value.as_str().and_then(parse_color) {
                    f.render_widget(
                        Paragraph::new(format!("■■ {} ▾", value.as_str().unwrap()))
                            .style(Style::new().fg(color)),
                        r,
                    );
                }
                return;
            }
            if values.len() > 3
                || total > r.width as usize
                || matches!(field.key, "theme" | "shell")
            {
                return;
            }
            let mut x = r.x;
            let current = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            for (i, v) in values.iter().enumerate() {
                let label = format!(
                    "{} {} ",
                    if current == *v { "●" } else { "○" },
                    option_label(field.key, v)
                );
                let w = label.width() as u16;
                let row = Rect::new(x, r.y, w, 1);
                f.render_widget(
                    Paragraph::new(label).style(if current == *v {
                        Style::new().fg(crate::appearance::palette().accent)
                    } else {
                        Style::default()
                    }),
                    row,
                );
                // Resolve by stable path, independent of the currently selected row.
                self.hits
                    .push((row, Hit::SetChoice(path.clone(), field.clone(), i)));
                x += w;
            }
        }
    }
}
pub(super) fn option_label(key: &str, value: &str) -> String {
    let pair = match (key, value) {
        (_, "$custom") => ("自定义…", "Custom…"),
        ("color", "") => ("默认颜色", "Default color"),
        ("shell", "pwsh") => ("PowerShell 7 (pwsh)", "PowerShell 7 (pwsh)"),
        ("shell", "powershell") => ("Windows PowerShell", "Windows PowerShell"),
        ("language", "auto") => ("跟随系统", "System"),
        ("language", "zh-cn") => ("简体中文", "简体中文"),
        ("language", "en-us") => ("English", "English"),
        ("new_window", "false") => ("当前窗口", "Current window"),
        ("new_window", "true") => ("新窗口", "New window"),
        ("new_instance", "false") => ("复用已有", "Reuse existing"),
        ("new_instance", "true") => ("始终新建", "Always create"),
        ("resume_latest", "true") => ("续接最近", "Resume latest"),
        ("resume_latest", "false") => ("每次新建", "Start fresh"),
        (_, "agent-only") => ("仅 Agent", "Agent only"),
        (_, "agent-debug") => ("Agent＋Debug", "Agent + Debug"),
        (_, "custom") => ("自定义", "Custom"),
        (_, "float") => ("浮层", "Floating"),
        (_, "split") => ("分屏", "Split"),
        (_, "tabs") => ("标签页", "Tabs"),
        (_, "terminal") => ("跟随终端", "Terminal"),
        (_, "tokyo") => ("Tokyo", "Tokyo"),
        (_, "catppuccin") => ("Catppuccin", "Catppuccin"),
        (_, "light") => ("浅色", "Light"),
        (_, "latest") => ("续接最近", "Resume latest"),
        (_, "fresh") => ("每次新建", "Start fresh"),
        (_, "picker") => ("会话选择器", "Conversation picker"),
        (_, "rendered") => ("渲染", "Rendered"),
        (_, "source") => ("源码", "Source"),
        (_, "auto") => ("自动", "Automatic"),
        (_, "halfblock") => ("半块字符", "Half-block characters"),
        _ => return value.into(),
    };
    tr(pair.0, pair.1)
}

fn parse_color(value: &str) -> Option<ratatui::style::Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(ratatui::style::Color::Rgb(
        (n >> 16) as u8,
        (n >> 8) as u8,
        n as u8,
    ))
}
