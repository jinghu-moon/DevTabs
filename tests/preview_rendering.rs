use devtabs::preview;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    widgets::Paragraph,
};
use std::{collections::HashSet, path::Path};

fn buffer(source: &str, name: &str, markdown: bool, width: u16) -> Buffer {
    let rendered = preview::render(source, Path::new(name), markdown, width, false);
    let mut terminal = Terminal::new(TestBackend::new(width, 50)).unwrap();
    terminal
        .draw(|f| {
            for (y, line) in rendered.text.lines.iter().take(50).enumerate() {
                f.render_widget(
                    Paragraph::new(line.clone()),
                    ratatui::layout::Rect::new(0, y as u16, width, 1),
                );
            }
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn toml_rust_json_and_powershell_have_multiple_token_colors() {
    for (name, source) in [
        (
            "Cargo.toml",
            "[package]\nname = \"devtabs\"\nversion = 3\nenabled = true\n# comment\n",
        ),
        (
            "main.rs",
            "fn main() { let answer = 42; println!(\"hello\"); }\n",
        ),
        (
            "package.json",
            "{\"name\": \"devtabs\", \"count\": 42, \"enabled\": true}\n",
        ),
        ("test.ps1", "$answer = 42\nWrite-Host \"hello\" # comment\n"),
    ] {
        let b = buffer(source, name, false, 80);
        let colors: HashSet<_> = b
            .content
            .iter()
            .filter(|c| !c.symbol().trim().is_empty())
            .map(|c| c.fg)
            .collect();
        assert!(colors.len() >= 3, "{name} lacks token colors: {colors:?}");
    }
}

#[test]
fn markdown_reading_hides_markup_and_preserves_heading_inline_and_code_styles() {
    let source = "# 中文标题\n\n## Section\n\n**strong** and *emphasis* and ~~gone~~ and `inline`\n\n[docs](https://example.com)\n\n```rust\nfn main() { let answer = 42; }\n```\n\n```unknown-language\nplain code\n```\n";
    let rendered = preview::render(source, Path::new("readme.md"), true, 18, false);
    assert!(
        !rendered
            .plain
            .iter()
            .any(|s| s.starts_with('#') || s.contains("```"))
    );
    let b = buffer(source, "readme.md", true, 18);
    let heading = b.content.iter().find(|c| c.symbol() == "中").unwrap();
    assert!(heading.modifier.contains(Modifier::BOLD));
    assert_ne!(heading.fg, Color::Reset);
    assert!(
        b.content
            .iter()
            .any(|c| c.modifier.contains(Modifier::ITALIC))
    );
    assert!(
        b.content
            .iter()
            .any(|c| c.modifier.contains(Modifier::CROSSED_OUT))
    );
    assert!(
        b.content
            .iter()
            .any(|c| c.modifier.contains(Modifier::UNDERLINED))
    );
    // Plain code still has a visible block background, including wrapped rows.
    let code = rendered
        .text
        .lines
        .iter()
        .find(|l| l.to_string().contains("plain code"))
        .unwrap();
    let mut block = Terminal::new(TestBackend::new(18, 1)).unwrap();
    block
        .draw(|f| f.render_widget(Paragraph::new(code.clone()), f.area()))
        .unwrap();
    assert_ne!(block.backend().buffer()[(0, 0)].bg, Color::Reset);
    let source_mode = preview::render(source, Path::new("readme.md"), false, 18, false);
    assert!(source_mode.plain[0].starts_with("# 中文标题"));
    assert!(source_mode.plain.iter().any(|s| s.starts_with("```rust")));
}

#[test]
fn wrapping_preserves_paragraph_styles_and_inline_overrides_on_every_row() {
    let source = "## long heading that wraps across rows\n\n> quote that wraps across several rows with **bold** text\n";
    let r = preview::render(source, Path::new("test.md"), true, 10, false);
    for (row, text) in r
        .plain
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.trim().is_empty())
    {
        let line = &r.text.lines[row];
        let mut t = Terminal::new(TestBackend::new(10, 1)).unwrap();
        t.draw(|f| f.render_widget(Paragraph::new(line.clone()), f.area()))
            .unwrap();
        let visible: Vec<_> = t
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|c| !c.symbol().trim().is_empty())
            .collect();
        assert!(
            visible.iter().all(|c| c.fg != Color::Reset),
            "lost style on wrapped row {row}: {text}"
        );
    }
}

#[test]
fn fenced_code_uses_shared_languages_and_keeps_nested_literal_content() {
    for light in [false, true] {
        for (language, code) in [
            ("toml", "[package]\nname = \"devtabs\"\nenabled = true"),
            ("powershell", "$answer = 42\nWrite-Host \"hello\" # comment"),
            ("rust", "fn main() { let answer = 42; }"),
        ] {
            let source = format!("```{language}\n{code}\n```\n");
            let r = preview::render(&source, Path::new("test.md"), true, 120, light);
            let colors: HashSet<_> = r
                .text
                .lines
                .iter()
                .flat_map(|l| &l.spans)
                .filter(|s| !s.content.trim().is_empty())
                .map(|s| s.style.fg)
                .collect();
            assert!(
                colors.len() >= 3,
                "{language} fenced code lacks highlighting"
            );
            assert!(r.plain.join("\n").contains(code));
        }
    }
    let source = "> before\n>\n> ```text\n> # literal heading\n>\n> **literal bold**\n> ```\n>\n> after\n\n- item\n\n  ```unknown\n  first\n\n  last\n  ```\n\n```\n```\n\nend\n";
    let r = preview::render(source, Path::new("nested.md"), true, 120, false);
    let plain = r.plain.join("\n");
    for expected in [
        "# literal heading",
        "**literal bold**",
        "first",
        "last",
        "after",
        "end",
    ] {
        assert!(plain.contains(expected), "missing {expected}: {plain}");
    }
    assert!(!plain.contains("devtabs-code-") && !plain.contains("```"));
    assert!(r.plain.windows(3).any(|rows| rows[0].contains("first")
        && rows[1].trim().is_empty()
        && rows[2].contains("last")));
}

#[test]
fn sidebar_async_preview_mode_switch_editor_and_search_keep_visible_styles() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use devtabs::sidebar::{Focus, Sidebar};
    use ratatui::{layout::Rect, style::Style, widgets::Block};
    use std::{
        fs,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!("DevTabs-render-ui-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let toml = root.join("Cargo.toml");
    let md = root.join("Preview.md");
    fs::write(&toml, "[package]\nname = \"devtabs\"\nversion = \"0.3.2\"\nenabled = true\n# TOML syntax highlighting\n").unwrap();
    fs::write(&md, "# DevTabs 阅读预览\n\n## 标题层级\n\n**粗体**、*斜体*、~~删除线~~ 和 `行内代码`\n\n[项目文档](https://example.com/docs)\n\n> 引用：标题和换行都应保留样式。\n\n```powershell\n$answer = 42\nWrite-Host \"Hello DevTabs\"\n```\n\n```toml\n[package]\nname = \"devtabs\"\n```\n\n| 功能 | 状态 |\n| --- | --- |\n| 预览 | 已启用 |\n").unwrap();
    let mut sidebar = Sidebar::new(root.clone());
    sidebar.visible = false;
    let area = Rect::new(0, 0, 100, 38);
    sidebar.update_layout(area, false);
    fn ready(sidebar: &mut Sidebar) {
        let start = Instant::now();
        loop {
            sidebar.tick(true);
            let d = sidebar.document.as_ref().unwrap();
            if !d.loading
                & d.rendered()
                    .as_ref()
                    .is_some_and(|r| r.markdown == d.markdown)
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn draw(sidebar: &mut Sidebar, area: Rect) -> Buffer {
        sidebar.update_layout(area, false);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
        terminal
            .draw(|f| {
                f.render_widget(
                    Block::default().style(
                        Style::new()
                            .fg(Color::Rgb(212, 212, 212))
                            .bg(Color::Rgb(16, 18, 23)),
                    ),
                    f.area(),
                );
                sidebar.draw(f);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }
    fn key(s: &mut Sidebar, c: char) {
        s.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .unwrap();
    }
    sidebar.open(toml.clone(), true).unwrap();
    ready(&mut sidebar);
    let toml_buffer = draw(&mut sidebar, area);
    let colors: HashSet<_> = toml_buffer
        .content
        .iter()
        .filter(|c| !c.symbol().trim().is_empty())
        .map(|c| c.fg)
        .collect();
    assert!(colors.len() >= 4);
    sidebar.open(md, true).unwrap();
    ready(&mut sidebar);
    let markdown_buffer = draw(&mut sidebar, area);
    assert!(
        markdown_buffer
            .content
            .iter()
            .any(|c| c.symbol() == "阅" && c.modifier.contains(Modifier::BOLD))
    );
    key(&mut sidebar, 'm');
    ready(&mut sidebar);
    assert!(
        !sidebar
            .document
            .as_ref()
            .unwrap()
            .rendered()
            .unwrap()
            .markdown
    );
    key(&mut sidebar, 'M');
    ready(&mut sidebar);
    assert_eq!(markdown_buffer, draw(&mut sidebar, area));
    key(&mut sidebar, '/');
    sidebar.paste("标题").unwrap();
    sidebar
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert!(!sidebar.document.as_ref().unwrap().matches.is_empty());
    assert!(
        draw(&mut sidebar, area)
            .content
            .iter()
            .any(|c| c.bg == Color::Yellow)
    );
    sidebar
        .key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();
    sidebar.open(toml, true).unwrap();
    ready(&mut sidebar);
    key(&mut sidebar, 'e');
    let started = Instant::now();
    loop {
        sidebar.tick(true);
        if sidebar
            .document
            .as_ref()
            .unwrap()
            .editor()
            .is_some_and(|e| e.highlight.is_some())
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sidebar.focus, Focus::Editor);
    let edit = draw(&mut sidebar, area);
    let colors: HashSet<_> = edit
        .content
        .iter()
        .filter(|c| !c.symbol().trim().is_empty())
        .map(|c| c.fg)
        .collect();
    assert!(colors.len() >= 4, "TOML editor must retain token colors");
    if let Some(directory) = std::env::var_os("DEVTABS_RENDER_SNAPSHOT") {
        let directory = std::path::PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        for (name, b) in [
            ("toml", toml_buffer),
            ("markdown", markdown_buffer),
            ("editor", edit),
        ] {
            let cells: Vec<_> = b.content.iter().map(|c| serde_json::json!({"text":c.symbol(),"fg":c.fg.to_string(),"bg":c.bg.to_string(),"bold":c.modifier.contains(Modifier::BOLD),"italic":c.modifier.contains(Modifier::ITALIC),"underline":c.modifier.contains(Modifier::UNDERLINED),"strike":c.modifier.contains(Modifier::CROSSED_OUT)})).collect();
            fs::write(
                directory.join(format!("{name}.json")),
                serde_json::to_vec(
                    &serde_json::json!({"width":b.area.width,"height":b.area.height,"cells":cells}),
                )
                .unwrap(),
            )
            .unwrap();
        }
    }
    drop(sidebar);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(windows)]
fn preview_retries_transient_sharing_but_reports_persistent_locks() {
    use std::{fs, os::windows::fs::OpenOptionsExt, sync::mpsc, time::Duration};
    let path = std::env::temp_dir().join(format!("DevTabs-read-lock-{}.txt", uuid::Uuid::new_v4()));
    fs::write(&path, "complete content").unwrap();
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(
        preview::read(&path).is_err(),
        "persistent locks must not be hidden"
    );
    let (tx, rx) = mpsc::channel();
    let target = path.clone();
    let worker = std::thread::spawn(move || {
        tx.send(()).unwrap();
        preview::read(&target)
    });
    rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(6));
    drop(locked);
    assert_eq!(
        worker.join().unwrap().unwrap().source.as_ref(),
        "complete content"
    );
    fs::remove_file(path).unwrap();
}
