//! Search, command table and binding inspector share the existing settings draft.
use super::*;
use ratatui::{
    layout::Position,
    style::{Color, Modifier},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

pub(super) fn text(zh: &'static str, en: &'static str) -> &'static str {
    if crate::i18n::language() == crate::i18n::Language::ZhCn {
        zh
    } else {
        en
    }
}
pub(super) fn context_label(context: &str) -> String {
    let name = match context {
        "overview" => text("总览", "Overview"),
        "overview.projects" => text("总览 · 项目列表", "Overview · Projects"),
        "overview.detail" => text("总览 · 详情", "Overview · Details"),
        "overview.statistics" => text("总览 · 统计", "Overview · Statistics"),
        "overview.tasks" => text("总览 · 任务", "Overview · Tasks"),
        "overview.queue" => text("总览 · 启动队列", "Overview · Queue"),
        "overview.search" => text("总览 · 搜索", "Overview · Search"),
        "app" => text("应用", "Application"),
        "workspace" => text("工作区", "Workspace"),
        "panel" => text("文件面板", "File panel"),
        "terminal.input" => text("终端输入区", "Terminal input"),
        "terminal.selection" => text("终端选区", "Terminal selection"),
        "tree" => text("文件树", "File tree"),
        "editor" => text("源码编辑器", "Editor"),
        "editor.search" => text("编辑器 · 查找", "Editor · Search"),
        "preview" => text("文件预览", "Preview"),
        "preview.image" => text("图片预览", "Image preview"),
        "preview.text" => text("文本预览", "Text preview"),
        "preview.markdown" => text("Markdown 预览", "Markdown preview"),
        "preview.diff" => text("差异预览", "Diff preview"),
        "dialog.diff" => text("文件差异", "Diff"),
        "dialog.search" => text("查找", "Search"),
        "dialog.info" => text("文件信息", "File information"),
        "dialog.bookmarks" => text("书签", "Bookmarks"),
        "dialog.outline" => text("大纲", "Outline"),
        "dialog.quick_open" => text("快速打开", "Quick open"),
        "dialog.options" => text("本次启动选项", "Launch options"),
        "dialog.presets" => text("预设", "Presets"),
        "dialog.queue" => text("启动队列", "Launch queue"),
        "dialog.instances" => text("项目实例", "Instances"),
        "dialog.history" => text("历史会话", "History"),
        "dialog.attention" => text("待处理事项", "Attention"),
        "dialog.palette" => text("命令面板", "Command palette"),
        "dialog.batch" => text("批量启动", "Batch launch"),
        "dialog.resources" => text("任务资源", "Task resources"),
        "dialog.notifications" => text("通知", "Notifications"),
        "dialog.review" => text("轮次审阅", "Round review"),
        "menu.terminals" => text("终端菜单", "Terminal menu"),
        "dialog.overview" => text("总览弹层", "Overview dialog"),
        "dialog.form" => text("表单", "Form"),
        "dialog.help" => text("帮助", "Help"),
        "gesture.move" => text("移动手势", "Move gesture"),
        "gesture.resize" => text("缩放手势", "Resize gesture"),
        "shortcuts.manage" => text("快捷键管理", "Shortcut manager"),
        "shortcuts.record" => text("快捷键录制", "Key recorder"),
        "shortcuts.test" => text("按键测试", "Key test"),
        _ => return context.into(),
    };
    name.into()
}
pub(super) fn label(id: &str) -> String {
    match id {
        "overview.note.edit" => text("编辑便签", "Edit note"),
        "overview.favorite" => text("收藏项目", "Toggle favorite"),
        "overview.settings" => text("设置", "Settings"),
        "overview.refresh" => text("刷新", "Refresh"),
        "overview.history" => text("历史会话", "Session history"),
        "overview.queue.launch" => text("启动队列", "Launch queue"),
        "overview.palette" => text("命令面板", "Command palette"),
        "shortcuts.open" => text("快捷键管理", "Keyboard shortcuts"),
        "editor.match.next" => text("下一处匹配", "Next match"),
        "editor.match.previous" => text("上一处匹配", "Previous match"),
        "editor.save" => text("保存文件", "Save file"),
        "workspace.layout.next" => text("切换布局", "Switch layout"),
        _ => return crate::shortcuts::command_label(id),
    }
    .into()
}
fn description(id: &str) -> &'static str {
    match id {
        "overview.note.edit" => text(
            "给当前项目写“下一步”，下次打开接着做。",
            "Write the next step for the selected project.",
        ),
        "overview.favorite" => text(
            "收藏或取消收藏选中的项目。",
            "Pin or unpin the selected project.",
        ),
        "overview.settings" | "settings.workspace.open" => text(
            "打开统一设置；全部分类共用一份草稿。",
            "Open settings; all categories share one draft.",
        ),
        "overview.refresh" => text(
            "重新读取项目与 Git 状态。",
            "Refresh projects and Git status.",
        ),
        "overview.history" => text(
            "浏览项目历史会话，暂存需要续接的会话。",
            "Browse project history and choose a conversation to resume.",
        ),
        "overview.queue.launch" => text(
            "执行已勾选并审阅的项目启动队列。",
            "Launch the reviewed queue of selected projects.",
        ),
        "overview.palette" => text(
            "搜索并执行当前可用的命令。",
            "Search and run available commands.",
        ),
        "shortcuts.open" => text(
            "打开此管理页；鼠标入口不依赖任何快捷键。",
            "Open this manager; mouse access works without a key binding.",
        ),
        "editor.match.next" => text("跳到下一个查找结果。", "Jump to the next search match."),
        "editor.match.previous" => {
            text("跳到上一个查找结果。", "Jump to the previous search match.")
        }
        "editor.save" => text(
            "保存当前文件；未保存保护仍然生效。",
            "Save the current file, preserving unsaved-change guards.",
        ),
        "workspace.layout.next" => text(
            "切换当前工作区的终端布局。",
            "Switch the terminal layout in this workspace.",
        ),
        "terminal.send_prefix" => text(
            "将原前缀按键发送给原终端；不重新触发命令解析。",
            "Send the original prefix key to its terminal.",
        ),
        _ => text(
            "在下列生效区域执行此命令。可用性由当前视图判断，管理页不会执行它。",
            "Runs in the scopes below when available in the active view. This manager never executes it.",
        ),
    }
}
fn key_color() -> Color {
    if crate::appearance::theme() == crate::appearance::Theme::Light {
        Color::Rgb(136, 82, 0)
    } else {
        Color::Rgb(240, 198, 116)
    }
}
fn clip(value: &str, width: u16) -> String {
    if value.width() <= width as usize {
        return value.into();
    }
    if width == 0 {
        return String::new();
    }
    format!("{}…", single_line(value, width.saturating_sub(1) as usize))
}
fn button(f: &mut Frame, area: Rect, label: &str, active: bool) {
    let p = crate::appearance::palette();
    f.render_widget(
        Paragraph::new(clip(&format!(" {label} "), area.width)).style(if active {
            Style::new()
                .fg(p.bg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(p.fg).bg(p.selected)
        }),
        area,
    );
}
fn flow(
    f: &mut Frame,
    area: Rect,
    items: Vec<(String, Action)>,
    hits: &mut Vec<(Rect, Action)>,
) -> u16 {
    let mut x = area.x;
    let mut y = area.y;
    for (label, action) in items {
        let width = (label.width() as u16 + 2).min(area.width);
        if x + width > area.right() {
            x = area.x;
            y += 1;
        }
        if y >= area.bottom() || width == 0 {
            break;
        }
        let r = Rect::new(x, y, width, 1);
        button(f, r, &label, false);
        hits.push((r, action));
        x += width + 1;
    }
    y.saturating_sub(area.y) + 1
}
fn panel(f: &mut Frame, area: Rect, title: String) -> Rect {
    let p = crate::appearance::palette();
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(p.accent))
        .style(crate::appearance::style());
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

#[derive(Default)]
struct Content {
    lines: Vec<Line<'static>>,
    hits: Vec<(usize, u16, u16, Action)>,
}
impl Content {
    fn text(&mut self, value: &str, width: u16, color: Color) {
        for line in wrap_lines(&crate::preview::safe_text(value), width.max(1) as usize) {
            self.lines.push(Line::styled(line, Style::new().fg(color)));
        }
    }
    fn buttons(&mut self, width: u16, items: Vec<(&str, Action)>) {
        let p = crate::appearance::palette();
        let mut line = Line::default();
        let mut x = 0;
        for (label, action) in items {
            let w = (label.width() as u16 + 2).min(width);
            if x + w > width {
                self.lines.push(line);
                line = Line::default();
                x = 0;
            }
            self.hits.push((self.lines.len(), x, w, action));
            line.spans.push(Span::styled(
                clip(&format!(" {label} "), w),
                Style::new().fg(p.fg).bg(p.selected),
            ));
            line.spans.push(Span::raw(" "));
            x += w + 1;
        }
        self.lines.push(line);
    }
    fn draw(self, f: &mut Frame, area: Rect, scroll: &mut u16, hits: &mut Vec<(Rect, Action)>) {
        *scroll = (*scroll).min(
            self.lines
                .len()
                .saturating_sub(area.height as usize)
                .min(u16::MAX as usize) as u16,
        );
        let first = *scroll as usize;
        for (row, x, width, action) in self.hits {
            if row >= first && row < first + area.height as usize && x < area.width {
                hits.push((
                    Rect::new(
                        area.x + x,
                        area.y + (row - first) as u16,
                        width.min(area.width - x),
                        1,
                    ),
                    action,
                ));
            }
        }
        f.render_widget(Paragraph::new(self.lines).scroll((*scroll, 0)), area);
    }
}
pub(super) fn transfer_actions() -> Vec<(&'static str, Action)> {
    vec![
        (
            text("导入快捷键 TOML（先预览）", "Import TOML (preview first)"),
            Action::Import,
        ),
        (
            text(
                "备份 / 导出已保存快捷键 TOML",
                "Back up / export saved shortcuts (TOML)",
            ),
            Action::Export,
        ),
        (
            text(
                "导出有效命令目录 JSON（仅查看）",
                "Export effective catalog (JSON, read only)",
            ),
            Action::ExportEffective,
        ),
    ]
}

impl Manager {
    pub fn focus_command(&mut self, id: &str, d: &Draft) {
        self.selected = self
            .commands(d)
            .iter()
            .position(|c| c.id == id)
            .unwrap_or(0);
        self.detail_scroll = 0;
        self.binding = 0;
    }
    pub fn set_context(&mut self, context: &str) {
        if let Some(i) = CONTEXTS.iter().position(|c| *c == context) {
            self.context = i;
        }
    }
    pub fn scroll_at(&mut self, point: Position, delta: i16, d: &Draft) {
        if self.exclusive() {
            if self.detail_area.contains(point) {
                match &mut self.mode {
                    Mode::Menu(i) => {
                        *i = i
                            .saturating_add_signed(delta as isize)
                            .min(menu_actions().len() - 1)
                    }
                    Mode::Transfers(i) => {
                        *i = i
                            .saturating_add_signed(delta as isize)
                            .min(transfer_actions().len() - 1)
                    }
                    _ => self.detail_scroll = self.detail_scroll.saturating_add_signed(delta),
                }
            }
            return;
        }
        if self.list_area.contains(point) {
            self.selected = self
                .selected
                .saturating_add_signed(delta as isize)
                .min(self.commands(d).len().saturating_sub(1));
            self.scope = 0;
            self.binding = 0;
            self.note_key = None;
            self.detail_scroll = 0;
        } else if self.detail_area.contains(point) {
            self.detail_scroll = self.detail_scroll.saturating_add_signed(delta);
        }
    }
    pub(super) fn draw_view(&mut self, f: &mut Frame, r: Rect, d: &Draft) -> Vec<(Rect, Action)> {
        let mut hits = vec![];
        self.list_area = Rect::default();
        self.detail_area = Rect::default();
        if r.width < 4 || r.height < 3 {
            return hits;
        }
        let p = crate::appearance::palette();
        let custom = settings(d).unwrap_or_default();
        let index = Index::load(&custom);
        if r.height < 18 && self.exclusive() {
            let inner = panel(f, r, text("编辑快捷键", "Edit shortcuts").into());
            self.detail_area = inner;
            self.draw_mode(f, inner, d, &index, &mut hits);
            return hits;
        }
        let commands = self.commands(d);
        self.selected = self.selected.min(commands.len().saturating_sub(1));
        let status = if d.dirty() {
            text("● 未保存", "● Unsaved")
        } else {
            text("✓ 已保存", "✓ Saved")
        };
        let status_width = (status.width() as u16).min(r.width.saturating_sub(4));
        let search = Rect::new(r.x, r.y, r.width.saturating_sub(status_width + 2), 1);
        f.render_widget(
            Paragraph::new(clip(
                &if self.search.is_empty() && !matches!(self.mode, Mode::Search) {
                    text("/ 查找命令、区域或按键…", "/ Find command, scope or key…").into()
                } else {
                    format!(
                        "/ {}{}",
                        self.search,
                        if matches!(self.mode, Mode::Search) {
                            "▏"
                        } else {
                            ""
                        }
                    )
                },
                search.width,
            ))
            .style(Style::new().bg(p.selected).fg(if self.search.is_empty() {
                p.muted
            } else {
                p.fg
            })),
            search,
        );
        f.render_widget(
            Paragraph::new(clip(status, status_width)).style(Style::new().fg(if d.dirty() {
                key_color()
            } else {
                p.accent
            })),
            Rect::new(search.right() + 1, r.y, status_width, 1),
        );
        if !self.exclusive() {
            hits.push((search, Action::Search));
        }
        let filters = [
            text("全部", "All"),
            text("当前区域", "Current scope"),
            text("已修改", "Modified"),
            text("未绑定", "Unbound"),
            text("内部冲突", "Conflicts"),
            text("含删除备注", "Deletion notes"),
        ];
        let mut x = r.x;
        let compact = r.height < 18;
        let mut y = r.y + 1;
        for (i, filter) in filters.iter().enumerate() {
            if compact && i != self.filter {
                continue;
            }
            let count = self.filtered_commands(&custom, &index, i, "").len();
            let label = format!("{filter} {count}");
            let width = (label.width() as u16 + 2).min(r.width);
            if x + width > r.right() {
                x = r.x;
                y += 1;
            }
            if y >= r.bottom().saturating_sub(3) {
                break;
            }
            let area = Rect::new(x, y, width, 1);
            button(f, area, &label, self.filter == i);
            if !self.exclusive() {
                hits.push((
                    area,
                    if compact {
                        Action::Filter
                    } else {
                        Action::FilterTo(i)
                    },
                ));
            }
            x += width + 1;
        }
        y += 1;
        let wide = r.width >= 108;
        if y < r.bottom() {
            let mut tools = vec![
                (text("更多操作", "More actions").to_string(), Action::Menu),
                (
                    format!("{} {}", text("前缀", "Prefix"), index.prefix),
                    Action::Prefix,
                ),
            ];
            if !wide {
                tools.insert(
                    0,
                    (
                        if self.detail {
                            text("← 命令列表", "← Commands")
                        } else {
                            text("当前详情 →", "Details →")
                        }
                        .into(),
                        if self.detail {
                            Action::BackToList
                        } else {
                            Action::Details
                        },
                    ),
                );
            }
            let mut tool_hits = vec![];
            let used = flow(
                f,
                Rect::new(r.x, y, r.width, (r.bottom() - y).min(2)),
                tools,
                &mut tool_hits,
            );
            if !self.exclusive() {
                hits.extend(tool_hits);
            }
            y += used;
        }
        let panels = Rect::new(
            r.x,
            y.min(r.bottom()),
            r.width,
            r.bottom().saturating_sub(y),
        );
        if panels.height < 3 {
            return hits;
        }
        let left_width = if wide { r.width * 3 / 5 } else { r.width };
        let list = Rect::new(panels.x, panels.y, left_width, panels.height);
        let detail = if wide {
            Rect::new(
                list.right() + 1,
                panels.y,
                panels.width - left_width - 1,
                panels.height,
            )
        } else {
            panels
        };
        if wide || (!self.detail && !self.exclusive()) {
            self.draw_commands(f, list, &custom, &index, &commands, &mut hits);
        }
        if wide || self.detail || self.exclusive() {
            let inner = panel(f, detail, text("当前命令", "Current command").into());
            self.detail_area = inner;
            if self.exclusive() {
                self.draw_mode(f, inner, d, &index, &mut hits);
            } else {
                self.draw_details(f, inner, d, &index, &mut hits);
            }
        }
        hits
    }
    fn draw_commands(
        &mut self,
        f: &mut Frame,
        area: Rect,
        custom: &model::Shortcuts,
        index: &Index,
        commands: &[crate::shortcuts::Command],
        hits: &mut Vec<(Rect, Action)>,
    ) {
        let inner = panel(
            f,
            area,
            format!("{} {}", text("命令", "Commands"), commands.len()),
        );
        self.list_area = inner;
        if inner.height == 0 || inner.width == 0 {
            return;
        }
        let source_width = if inner.width >= 60 { 8 } else { 0 };
        let context_width = if inner.width >= 45 { 12 } else { 0 };
        let key_width = (inner.width * 2 / 5).max(12).min(
            inner
                .width
                .saturating_sub(source_width + context_width + 10),
        );
        let name_width = inner
            .width
            .saturating_sub(source_width + context_width + key_width);
        let columns = [
            (0, name_width),
            (name_width, context_width),
            (name_width + context_width, key_width),
            (inner.width - source_width, source_width),
        ];
        for ((x, w), title) in columns.iter().zip([
            text("功能", "Command"),
            text("区域", "Scope"),
            text("绑定", "Bindings"),
            text("来源", "Source"),
        ]) {
            f.render_widget(
                Paragraph::new(clip(title, *w)).style(Style::new().fg(palette().muted)),
                Rect::new(inner.x + x, inner.y, *w, 1),
            );
        }
        if commands.is_empty() {
            f.render_widget(
                Paragraph::new(text(
                    "没有匹配的命令。清空搜索或切换筛选。",
                    "No matching commands. Clear search or change the filter.",
                ))
                .wrap(Wrap { trim: true }),
                Rect::new(
                    inner.x,
                    inner.y + 1,
                    inner.width,
                    inner.height.saturating_sub(1),
                ),
            );
            return;
        }
        let first = self
            .selected
            .saturating_sub(inner.height.saturating_sub(2) as usize);
        let p = palette();
        for (i, c) in commands
            .iter()
            .enumerate()
            .skip(first)
            .take(inner.height.saturating_sub(1) as usize)
        {
            let row = Rect::new(inner.x, inner.y + 1 + (i - first) as u16, inner.width, 1);
            if self.selected == i {
                f.render_widget(Block::default().style(Style::new().bg(p.selected)), row);
            }
            let binding = index.binding(c.id, c.context);
            let keys = binding
                .filter(|b| !b.disabled)
                .map(|b| {
                    let preferred = index.preferred(c.id, c.context);
                    let mut keys = b.keys.iter().map(Sequence::display).collect::<Vec<_>>();
                    keys.sort_by_key(|s| s != &preferred);
                    keys.join(" / ")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| text("未绑定", "Unbound").into());
            let custom = custom.bindings.iter().any(|b| b.command == c.id);
            let values = [
                format!(
                    "{}{}",
                    if self.selected == i { "▏" } else { " " },
                    label(c.id)
                ),
                context_label(c.context),
                keys,
                text(
                    if custom { "自定义" } else { "默认" },
                    if custom { "Custom" } else { "Default" },
                )
                .into(),
            ];
            for (n, ((x, w), value)) in columns.iter().zip(values).enumerate() {
                f.render_widget(
                    Paragraph::new(clip(&value, w.saturating_sub(1))).style(Style::new().fg(
                        if n == 2 {
                            key_color()
                        } else if n == 1 || n == 3 {
                            p.muted
                        } else {
                            p.fg
                        },
                    )),
                    Rect::new(row.x + x, row.y, *w, 1),
                );
            }
            if !self.exclusive() {
                hits.push((row, Action::Select(i)));
            }
        }
    }
    fn draw_details(
        &mut self,
        f: &mut Frame,
        area: Rect,
        d: &Draft,
        index: &Index,
        hits: &mut Vec<(Rect, Action)>,
    ) {
        let Some(c) = self.command(d) else {
            return;
        };
        let mut content = Content::default();
        let p = palette();
        let custom = settings(d).unwrap_or_default();
        let edit = custom
            .bindings
            .iter()
            .find(|b| b.command == c.id && b.context == c.context);
        let declared = index.commands.iter().find(|b| b.id == c.id).unwrap();
        let disabled = index.binding(c.id, c.context).is_some_and(|b| b.disabled);
        content.text(&label(c.id), area.width, p.accent);
        content.text(c.id, area.width, p.muted);
        content.text(description(c.id), area.width, p.fg);
        content.text(
            &format!(
                "{}: {}",
                text("生效区域", "Scope"),
                context_label(c.context)
            ),
            area.width,
            p.muted,
        );
        if command_contexts(c.id, declared.context).len() > 1 {
            content.buttons(
                area.width,
                vec![(text("切换作用域", "Change scope"), Action::Scope)],
            );
        }
        content.text("", area.width, p.fg);
        content.text(text("绑定", "Bindings"), area.width, p.accent);
        let keys = self.effective_keys(d);
        if keys.is_empty() {
            content.text(
                text(
                    "未绑定；仍可从鼠标菜单执行。",
                    "Unbound; mouse commands remain available.",
                ),
                area.width,
                p.muted,
            );
        }
        for (i, key) in keys.iter().enumerate() {
            let preferred = index.preferred(c.id, c.context) == key.display();
            content.text(
                &format!(
                    "{}{}{}",
                    if i == self.binding { "▸ " } else { "  " },
                    key.display(),
                    if disabled {
                        text("（已停用）", " (disabled)")
                    } else if preferred {
                        text(" · 首选显示", " · Preferred")
                    } else {
                        ""
                    }
                ),
                area.width,
                if disabled { p.muted } else { key_color() },
            );
            let mut buttons = vec![
                (text("改", "Edit"), Action::ModifyKey(i)),
                (text("删", "Delete"), Action::Remove(i)),
                (text("删并备注", "Delete + note"), Action::RemoveNote(i)),
            ];
            if !disabled && !preferred {
                buttons.push((text("设为首选", "Prefer"), Action::Prefer(i)));
            }
            content.buttons(area.width, buttons);
        }
        content.text("", area.width, p.fg);
        content.buttons(
            area.width,
            vec![
                (text("+ 添加绑定", "+ Add binding"), Action::Add),
                (text("恢复此命令", "Restore command"), Action::Restore),
                (text("按键测试", "Key test"), Action::Test),
            ],
        );
        content.text("", area.width, p.fg);
        for key in &keys {
            if let Some(other) = index.bindings.iter().find(|b| {
                b.command != c.id
                    && !b.disabled
                    && b.keys.contains(key)
                    && !crate::shortcuts::contexts_overlap(&b.context, c.context)
            }) {
                content.text(
                    &format!(
                        "{}: {} → {} ({})",
                        text("同键复用，区域不同，允许", "Allowed reuse in another scope"),
                        key.display(),
                        label(&other.command),
                        context_label(&other.context)
                    ),
                    area.width,
                    p.muted,
                );
            }
        }
        let source = if edit.is_some() {
            text("自定义差异", "custom overrides")
        } else if c.context != declared.context {
            text("继承父区域", "inherited")
        } else {
            text("默认", "Default")
        };
        for value in [
            format!(
                "{}: {}",
                text("默认", "Default"),
                declared
                    .defaults
                    .iter()
                    .map(Sequence::display)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
            format!("{}: {source}", text("来源", "Source")),
            format!(
                "{}: {}",
                text("状态", "Status"),
                if disabled {
                    text("已停用", "Disabled")
                } else if keys.is_empty() {
                    text("未绑定", "Unbound")
                } else {
                    text("已启用", "Enabled")
                }
            ),
            format!(
                "{}: {}",
                text("允许区域", "Allowed"),
                command_contexts(c.id, declared.context)
                    .into_iter()
                    .map(context_label)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ),
        ] {
            content.text(&value, area.width, p.muted);
        }
        if let Some(edit) = edit {
            for (i, key) in edit.remove.iter().enumerate() {
                let key = Sequence::parse(key, index.prefix)
                    .map(|s| s.display())
                    .unwrap_or_default();
                content.text(
                    &format!("{}: {key}", text("已删除", "Removed")),
                    area.width,
                    p.muted,
                );
                content.buttons(
                    area.width,
                    vec![(text("恢复此绑定", "Restore binding"), Action::RestoreKey(i))],
                );
            }
            for (key, note) in &edit.notes {
                content.text(
                    &format!("{}: {key} · {note}", text("删除备注", "Notes")),
                    area.width,
                    p.muted,
                );
            }
        }
        if self.note_key.is_some() {
            content.buttons(
                area.width,
                vec![
                    (text("编辑删除备注", "Edit deletion note"), Action::Note),
                    (text("清除此备注", "Clear note"), Action::ClearNote),
                ],
            );
        }
        for diagnostic in index.diagnostics.iter().filter(|s| s.contains(c.id)) {
            content.text(diagnostic, area.width, p.error);
        }
        if let Some(error) = &self.error {
            content.text(error, area.width, p.error);
        }
        if !self.message.is_empty() {
            content.text(&self.message, area.width, p.muted);
        }
        content.text(
            &format!(
                "{}: {}",
                text("终端本地占用", "Terminal-owned keys"),
                index.terminal_owned().join(", ")
            ),
            area.width,
            p.muted,
        );
        if content.lines.len() > area.height as usize && area.height > 2 {
            let body = Rect::new(area.x, area.y, area.width, area.height - 1);
            content.draw(f, body, &mut self.detail_scroll, hits);
            flow(
                f,
                Rect::new(area.x, area.bottom() - 1, area.width, 1),
                vec![
                    (text("↑ 上滚", "↑ Up").into(), Action::Scroll(-3)),
                    (text("↓ 下滚", "↓ Down").into(), Action::Scroll(3)),
                ],
                hits,
            );
        } else {
            content.draw(f, area, &mut self.detail_scroll, hits);
        }
    }
    fn draw_mode(
        &mut self,
        f: &mut Frame,
        area: Rect,
        d: &Draft,
        index: &Index,
        hits: &mut Vec<(Rect, Action)>,
    ) {
        let p = palette();
        if let Mode::Menu(selected)
        | Mode::Transfers(selected)
        | Mode::Removed(selected)
        | Mode::Inactive(selected) = self.mode
        {
            let title = match self.mode {
                Mode::Menu(_) => text("更多操作", "More actions"),
                Mode::Transfers(_) => text("备份 / 导入 / 导出", "Backup / import / export"),
                Mode::Removed(_) => text("已删除绑定", "Removed bindings"),
                _ => text("失效配置", "Inactive entries"),
            };
            let rows = match self.mode {
                Mode::Menu(_) => menu_actions()
                    .into_iter()
                    .map(|(s, a)| (s.into(), a))
                    .collect::<Vec<_>>(),
                Mode::Transfers(_) => transfer_actions()
                    .into_iter()
                    .map(|(s, a)| (s.into(), a))
                    .collect(),
                Mode::Removed(_) => self
                    .command(d)
                    .and_then(|c| {
                        settings(d).ok().and_then(|s| {
                            s.bindings
                                .into_iter()
                                .find(|b| b.command == c.id && b.context == c.context)
                        })
                    })
                    .map(|b| {
                        b.remove
                            .iter()
                            .enumerate()
                            .map(|(i, key)| {
                                (
                                    Sequence::parse(key, index.prefix)
                                        .map(|s| s.display())
                                        .unwrap_or_default(),
                                    Action::RestoreKey(i),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => settings(d)
                    .unwrap_or_default()
                    .bindings
                    .iter()
                    .filter(|b| {
                        index
                            .commands
                            .iter()
                            .find(|c| c.id == b.command)
                            .is_none_or(|c| {
                                !command_contexts(c.id, c.context).contains(&b.context.as_str())
                            })
                    })
                    .enumerate()
                    .map(|(i, b)| {
                        (
                            format!("{} [{}] · {}", b.command, b.context, text("删除", "Delete")),
                            Action::DeleteInactive(i),
                        )
                    })
                    .collect(),
            };
            f.render_widget(
                Paragraph::new(title).style(Style::new().fg(p.accent)),
                Rect::new(area.x, area.y, area.width, area.height.min(1)),
            );
            let visible = area.height.saturating_sub(3) as usize;
            let first = selected.saturating_sub(visible.saturating_sub(1));
            for (i, (name, action)) in rows.into_iter().enumerate().skip(first).take(visible) {
                let row = Rect::new(area.x, area.y + 1 + (i - first) as u16, area.width, 1);
                button(f, row, &name, i == selected);
                hits.push((row, action));
            }
            if area.height > 1 {
                flow(
                    f,
                    Rect::new(area.x, area.bottom() - 1, area.width, 1),
                    vec![(text("← 返回", "← Back").into(), Action::Cancel)],
                    hits,
                );
            }
            return;
        }
        let mut content = Content::default();
        let title = if self.candidate.is_some() {
            text("确认草稿", "Review draft")
        } else {
            match self.mode {
                Mode::Record(_) => text("录制绑定", "Record binding"),
                Mode::Test { .. } => text("按键测试", "Key test"),
                Mode::Prefix(_) => text("修改前缀", "Change prefix"),
                Mode::Note(_) => text("删除备注", "Deletion note"),
                Mode::Import(_) => text("导入文件路径", "Import file path"),
                Mode::Export(_) => text("导出文件路径", "Export file path"),
                _ => text("文本输入绑定", "Enter binding"),
            }
        };
        content.text(title, area.width, p.accent);
        if self.candidate.is_some() || self.export_confirm.is_some() {
            content.text(&self.message, area.width, key_color());
        } else {
            match &self.mode {
            Mode::Record(keys) => {
                content.text(text("直接按下要绑定的键；松开后形成候选。", "Press a key, then release it to capture a candidate."),area.width,p.muted);
                content.text(&keys.iter().map(ToString::to_string).collect::<Vec<_>>().join(" → "),area.width,key_color());
                content.text(text("Esc / Enter 也会录制，请点击完成或取消。", "Esc / Enter are recorded too. Click Done or Cancel."),area.width,p.muted);
            }
            Mode::Test{received} => {
                content.text(&format!("{}: {}",text("模拟区域", "Test scope"),context_label(CONTEXTS[self.context])),area.width,p.muted);
                content.buttons(area.width,["overview","editor","tree","terminal.input"].iter().map(|context| (match *context { "overview" => text("总览", "Overview"), "editor"=>text("编辑器", "Editor"), "tree"=>text("文件树", "Tree"), _=>text("终端", "Terminal") }, Action::SetContext(CONTEXTS.iter().position(|c|c==context).unwrap()))).collect());
                content.text(text("不执行命令、不发给终端；外部全局热键仍可能触发。", "No commands or terminal input. External global hotkeys may still run."),area.width,p.muted);
                if received.is_empty() { content.text(text("尚未收到按键。未收到不代表已知外部占用。", "No key received yet. This does not prove an external conflict."),area.width,p.muted); }
                for line in received.iter().rev().take(4) { content.text(line,area.width,key_color()); }
                if let Some(raw) = self.raw_events.back() { content.text(raw,area.width,p.muted); }
            }
            Mode::Text(_) => content.text(text("输入组合键或两步序列，例如 Ctrl+F3 或 Ctrl+G -> N。", "Enter a chord or two-step sequence: Ctrl+F3 or Ctrl+G -> N."),area.width,p.muted),
            Mode::Note(_) => content.text(text("用户备注不会参与按键匹配；留空可清除备注。", "User notes never affect matching. Leave empty to clear the note."),area.width,p.muted),
            Mode::Export(_) => content.text(text("导出已保存配置；草稿不提前写入。覆盖文件前需确认。", "Export saved bindings. Drafts stay unsaved; replacement requires confirmation."),area.width,p.muted),
            _ => {}
        }
        }
        if let Some(error) = &self.error {
            content.text(error, area.width, p.error);
        }
        let mut buttons = if self.candidate.is_some() {
            vec![
                (text("确认草稿", "Confirm draft"), Action::ConfirmRisk),
                (text("取消", "Cancel"), Action::Cancel),
            ]
        } else if self.export_confirm.is_some() {
            vec![
                (text("确认导出", "Confirm export"), Action::ConfirmExport),
                (text("取消", "Cancel"), Action::Cancel),
            ]
        } else if matches!(self.mode, Mode::Test { .. }) {
            let mut v = vec![
                (text("返回命令", "Back"), Action::Cancel),
                (text("切换测试区域", "Change scope"), Action::Context),
                (text("清空记录", "Clear log"), Action::ClearTest),
            ];
            if self.last_test.is_some() {
                v.push((
                    text("设为首选显示", "Prefer tested key"),
                    Action::PreferTest,
                ));
            }
            v
        } else {
            let mut v = vec![
                (text("完成", "Done"), Action::Finish),
                (text("取消", "Cancel"), Action::Cancel),
            ];
            if matches!(self.mode, Mode::Record(_)) {
                v.push((text("文本输入", "Enter text"), Action::Text));
            }
            v
        };
        if self.conflict.is_some() {
            buttons.push((
                text("替换冲突", "Replace conflict"),
                Action::ReplaceConflict,
            ));
        }
        let mut button_hits = vec![];
        let footer_h = if area.width < 46 { 3 } else { 2 }.min(area.height);
        let foot = Rect::new(
            area.x,
            area.bottom().saturating_sub(footer_h),
            area.width,
            footer_h,
        );
        let body = Rect::new(
            area.x,
            area.y,
            area.width,
            area.height.saturating_sub(footer_h),
        );
        let has_input = self.candidate.is_none()
            && self.export_confirm.is_none()
            && matches!(
                self.mode,
                Mode::Text(_) | Mode::Prefix(_) | Mode::Note(_) | Mode::Import(_) | Mode::Export(_)
            );
        let input_height = if has_input { body.height.min(2) } else { 0 };
        let text_area = Rect::new(body.x, body.y, body.width, body.height - input_height);
        content.draw(f, text_area, &mut self.detail_scroll, hits);
        if has_input {
            if let Mode::Text(a)
            | Mode::Prefix(a)
            | Mode::Note(a)
            | Mode::Import(a)
            | Mode::Export(a) = &self.mode
            {
                let input = Rect::new(
                    body.x,
                    body.bottom() - input_height,
                    body.width,
                    input_height,
                );
                f.render_widget(a, input);
            }
        }
        flow(
            f,
            foot,
            buttons.into_iter().map(|(s, a)| (s.into(), a)).collect(),
            &mut button_hits,
        );
        hits.extend(button_hits);
    }
}
fn palette() -> crate::appearance::Palette {
    crate::appearance::palette()
}
