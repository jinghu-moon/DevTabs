//! The settings form's field catalogue. Compound values retain their actual types.
use toml::Value;

#[derive(Clone, Debug)]
pub enum Kind {
    Toggle {
        inverse: bool,
    },
    Choice(Vec<&'static str>),
    Number {
        min: i64,
        max: i64,
        step: i64,
        percent: bool,
    },
    Text {
        multiline: bool,
        path: bool,
    },
    Object(Vec<Field>),
    List {
        item: Box<Field>,
        ordered: bool,
    },
    Map,
    Shortcuts,
    Actions,
}
#[derive(Clone, Debug)]
pub struct Field {
    pub key: &'static str,
    pub zh: &'static str,
    pub en: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    pub default: Value,
    pub optional: bool,
    pub readonly: bool,
}
impl Field {
    pub fn label(&self) -> &str {
        if crate::i18n::language() == crate::i18n::Language::ZhCn {
            self.zh
        } else {
            self.en
        }
    }
    pub fn object(
        key: &'static str,
        zh: &'static str,
        en: &'static str,
        fields: Vec<Field>,
    ) -> Self {
        Self {
            key,
            zh,
            en,
            help: "",
            kind: Kind::Object(fields),
            default: Value::Table(Default::default()),
            optional: false,
            readonly: false,
        }
    }
    fn scalar(
        key: &'static str,
        zh: &'static str,
        en: &'static str,
        kind: Kind,
        default: Value,
    ) -> Self {
        Self {
            key,
            zh,
            en,
            help: "",
            kind,
            default,
            optional: true,
            readonly: false,
        }
    }
    fn help(mut self, help: &'static str) -> Self {
        self.help = help;
        self
    }
    fn required(mut self) -> Self {
        self.optional = false;
        self
    }
    fn frozen(mut self) -> Self {
        self.readonly = true;
        self
    }
}
fn boolean(key: &'static str, zh: &'static str, en: &'static str, v: bool) -> Field {
    Field::scalar(key, zh, en, Kind::Toggle { inverse: false }, v.into())
}
fn choice(
    key: &'static str,
    zh: &'static str,
    en: &'static str,
    values: &[&'static str],
    v: &str,
) -> Field {
    Field::scalar(key, zh, en, Kind::Choice(values.to_vec()), v.into())
}
fn text(
    key: &'static str,
    zh: &'static str,
    en: &'static str,
    v: &str,
    multiline: bool,
    path: bool,
) -> Field {
    Field::scalar(key, zh, en, Kind::Text { multiline, path }, v.into())
}
fn number(
    key: &'static str,
    zh: &'static str,
    en: &'static str,
    v: i64,
    min: i64,
    max: i64,
    step: i64,
    percent: bool,
) -> Field {
    Field::scalar(
        key,
        zh,
        en,
        Kind::Number {
            min,
            max,
            step,
            percent,
        },
        v.into(),
    )
}
fn list(
    key: &'static str,
    zh: &'static str,
    en: &'static str,
    item: Field,
    ordered: bool,
) -> Field {
    Field::scalar(
        key,
        zh,
        en,
        Kind::List {
            item: Box::new(item),
            ordered,
        },
        Value::Array(vec![]),
    )
}
fn argv(key: &'static str, zh: &'static str, en: &'static str) -> Field {
    list(key,zh,en,text("", "字面参数", "Literal argument", "",false,false).required(),true)
        .help("每行是一个字面参数；空串也是一个参数。One literal argument per row; empty entries are retained.")
}
fn sessions() -> Field {
    list("sessions","额外会话","Extra sessions",Field::object("","会话","Session",vec![
        text("name","名称","Name","Session",false,false).required(),
        text("directory","启动目录","Directory","",false,true).help("相对 Task.cwd；空值使用任务工作目录。Relative to Task.cwd; empty uses Task.cwd."),
        text("command","启动命令","Command","",true,false).help("只编辑文本，空值只打开 PowerShell。Text only; empty opens PowerShell."),
    ]),true)
}
fn launch(snapshot: bool, project: bool) -> Field {
    let mut fields = vec![
        choice(
            "agent",
            "默认工具",
            "Agent",
            &["codex", "claude", "shell"],
            "codex",
        ),
        choice(
            "layout",
            "终端布局",
            "Layout",
            &["float", "split", "tabs"],
            "float",
        ),
        choice(
            "composition",
            "会话组合",
            "Composition",
            &["agent-only", "agent-debug", "custom"],
            "agent-debug",
        ),
    ];
    if snapshot {
        fields.push(choice(
            "conversation",
            "会话策略",
            "Conversation",
            &["latest", "fresh", "picker"],
            "latest",
        ));
        fields.push(bool_choice(
            "new_instance",
            "已有工作区",
            "Existing workspace",
            false,
        ));
        let mut profile = boolean(
            "no_profile",
            "加载个人 Profile",
            "Load PowerShell Profile",
            false,
        );
        profile.kind = Kind::Toggle { inverse: true };
        fields.push(profile);
    } else {
        let mut resume = choice(
            "resume_latest",
            "会话行为",
            "Conversation",
            &["true", "false"],
            "true",
        );
        resume.default = true.into();
        fields.push(resume);
    }
    fields.extend([
        bool_choice("new_window", "打开位置", "Open location", false),
        argv("agent_args", "Agent 参数", "Agent arguments"),
        text(
            "debug_command",
            "Debug 启动命令",
            "Debug command",
            "",
            true,
            false,
        ),
        sessions(),
        choice("shell", "PowerShell 程序", "PowerShell program", &["pwsh", "powershell", "$custom"], "pwsh")
            .help("候选来自标准程序名；可输入绝对路径，探测仅检查存在，不执行。Choose a program or custom absolute path; probing never executes it."),
    ]);
    if snapshot || project {
        fields.extend([
            text("title", "自定义标题", "Custom title", "", false, false),
            choice(
                "color",
                "标签颜色",
                "Tab color",
                &[
                    "", "#58a6ff", "#3fb950", "#d29922", "#f85149", "#bc8cff", "$custom",
                ],
                "",
            ),
        ]);
    }
    if snapshot {
        for f in &mut fields {
            if !matches!(f.key, "title" | "color") {
                f.optional = false;
            }
        }
    }
    Field::object("launch", "启动默认", "Launch", fields)
}
fn sidebar() -> Field {
    Field::object(
        "sidebar",
        "文件树默认",
        "File defaults",
        vec![
            number(
                "width",
                "文件树宽度（列）",
                "Tree width (columns)",
                28,
                20,
                80,
                1,
                false,
            ),
            number(
                "agent_ratio",
                "Agent 区域比例",
                "Agent area ratio",
                5000,
                1,
                9999,
                100,
                true,
            ),
            boolean("follow", "跟随文件选择", "Follow selection", true),
            boolean(
                "compact",
                "合并单分支目录",
                "Compact single-child directories",
                true,
            ),
            boolean(
                "show_hidden",
                "默认显示隐藏文件",
                "Show hidden files",
                false,
            ),
            boolean(
                "show_ignored",
                "默认显示忽略文件",
                "Show ignored files",
                false,
            ),
        ],
    )
}
fn check() -> Field {
    let mut args = argv(
        "argv",
        "程序与参数（首项为程序）",
        "Program and arguments (program first)",
    )
    .required();
    args.default = Value::Array(vec!["".into()]);
    Field::object("check","默认检查","Default check",vec![
        args,
        text("cwd","工作目录","Working directory",".",false,true).required().help("相对任务 checkout，读取与保存不执行。Relative to task checkout; editing never executes."),
        number("timeout_ms","超时（毫秒）","Timeout (milliseconds)",30000,1,3600000,1000,false).required(),
        Field::scalar("env","环境变量","Environment",Kind::Map,Value::Table(Default::default())).required(),
    ])
}
pub fn project() -> Field {
    let mut c = check();
    c.optional = true;
    Field::object(
        "",
        "项目",
        "Project",
        vec![
            text("path", "项目路径", "Project path", "", false, true)
                .required()
                .frozen(),
            boolean("favorite", "收藏", "Favorite", false),
            launch(false, true),
            sidebar(),
            Field::object(
                "checks",
                "准备与检查",
                "Prepare and check",
                vec![
                    list(
                        "prepare",
                        "任务准备步骤",
                        "Preparation steps",
                        check(),
                        true,
                    ),
                    c,
                ],
            ),
        ],
    )
}
pub fn snapshot() -> Field {
    let mut terminal = number(
        "history_lines",
        "历史行数",
        "History lines",
        5000,
        0,
        20000,
        500,
        false,
    );
    terminal.optional = false;
    Field::object(
        "",
        "启动快照",
        "Launch snapshot",
        vec![
            text("project", "启动项目", "Project", "", false, true).required(),
            launch(true, true),
            Field::object("terminal", "终端", "Terminal", vec![terminal]),
        ],
    )
}
/// Page order and field ownership are declared here; no effective defaults are written on browse.
pub fn pages() -> Vec<Field> {
    vec![
        Field::object("general","通用","General",vec![choice("language","界面语言","Language",&["auto","zh-cn","en-us"],"auto")]),
        Field::object("appearance","外观","Appearance",vec![choice("theme","界面主题","Theme",&["terminal","tokyo","catppuccin","light"],"terminal")]),
        Field::object("overview","总览","Overview",vec![
            choice("sort","默认排序","Default sort",&["recent","name","size","activity"],"recent"),
            choice("filter","默认筛选","Default filter",&["all","running","dirty","attention","pending","codex","claude","shell"],"all"),
            boolean("detail","详情面板","Details panel",true),boolean("statistics","统计面板","Statistics panel",true),
            number("list_ratio","项目列表比例","Project list ratio",3800,1,9999,100,true),
            number("detail_ratio","详情区域比例","Detail area ratio",3800,1,9999,100,true),
        ]),
        Field::object("discovery","项目发现","Discovery",["roots","added","excluded"].into_iter().map(|key| {
            let (zh,en,help)=match key {
                "roots"=>("项目根目录","Project roots","只发现直接子目录。Discovers direct child directories only."),
                "added"=>("手动加入项目","Added projects","每条为一个项目的绝对路径。Each entry is one absolute project path."),
                _=>("排除项目","Excluded projects","按规范路径精确排除，不排除子树。Exact normalized project paths; not subtrees."),
            };
            list(key,zh,en,text("","路径","Path","",false,true).required(),false).help(help)
        }).collect()),
        launch(false,false),
        Field::object("terminal","终端","Terminal",vec![number("history_lines","终端历史行数","History lines",5000,0,20000,500,false).help("仅新建 PTY 采用；0 不保留历史。New PTYs only; zero disables scrollback.")]),
        sidebar(),
        Field::object("preview","文件预览","Preview",vec![boolean("wrap","默认自动换行","Wrap by default",false),choice("markdown_mode","Markdown 默认模式","Markdown mode",&["rendered","source"],"rendered"),choice("image_protocol","图片显示方式","Image protocol",&["auto","sixel","halfblock"],"auto").help("偏好不代表终端支持。Preference does not imply terminal support.")]),
        list("project","项目设置","Projects",project(),false),
        list("preset","启动预设","Launch presets",Field::object("","预设","Preset",vec![text("name","预设名称","Name","",false,false).required(),list("entries","项目启动顺序","Launch order",snapshot(),true).required()]),false),
        Field::scalar("shortcuts","快捷键","Shortcuts",Kind::Shortcuts,Value::Table(Default::default())).required(),
        Field::object("notifications","通知与集成","Notifications / Hooks",vec![
            boolean("enabled","系统通知","System notifications",true),boolean("sound","通知声音","Notification sound",true),
            Field::scalar("$integrations","宿主采用 / Hooks / 通知注册","Host adoption / Hooks / registration",Kind::Actions,Value::Table(Default::default())).required(),
            Field::object("categories","通知类别","Categories",vec![boolean("end","完成","Completion",true),boolean("failure","失败","Failure",true),boolean("permission","权限请求","Permission request",true),boolean("input","等待输入","Waiting for input",true)]),
        ]),
        Field::scalar("data","配置与数据","Configuration / data",Kind::Actions,Value::Table(Default::default())).required(),
    ]
}

fn bool_choice(key: &'static str, zh: &'static str, en: &'static str, value: bool) -> Field {
    let mut f = choice(key, zh, en, &["false", "true"], "false");
    f.default = value.into();
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::model::*;
    fn keys<T: serde::Serialize>(value: T) -> std::collections::BTreeSet<String> {
        serde_json::to_value(value)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }
    fn names(field: &Field) -> std::collections::BTreeSet<String> {
        let Kind::Object(fields) = &field.kind else {
            panic!("object")
        };
        fields
            .iter()
            .filter(|f| !f.key.starts_with('$'))
            .map(|f| f.key.to_owned())
            .collect()
    }
    fn find<'a>(field: &'a Field, key: &str) -> &'a Field {
        let Kind::Object(fields) = &field.kind else {
            panic!("object")
        };
        fields.iter().find(|f| f.key == key).unwrap()
    }
    #[test]
    fn catalogue_covers_real_model_fields_and_preserves_scalar_types() {
        let p = pages();
        let root = Field::object("", "", "", p.clone());
        let mut expected = keys(Preferences::default());
        expected.remove("schema");
        let mut actual = names(&root);
        actual.remove("data");
        assert_eq!(actual, expected);
        for (key, expected) in [
            ("general", keys(General::default())),
            ("appearance", keys(Appearance::default())),
            ("overview", keys(Overview::default())),
            ("discovery", keys(Discovery::default())),
            ("terminal", keys(Terminal::default())),
            ("sidebar", keys(Sidebar::default())),
            ("preview", keys(Preview::default())),
            ("notifications", keys(Notifications::default())),
        ] {
            assert_eq!(names(find(&root, key)), expected, "{key}");
        }
        let project = project();
        assert_eq!(
            names(find(&project, "launch")),
            keys(LaunchOverrides::default())
        );
        assert_eq!(names(find(&project, "checks")), keys(Checks::default()));
        let snapshot = snapshot();
        let mut launch = LaunchValues::default();
        launch.title = Some("".into());
        launch.color = Some("".into());
        assert_eq!(names(find(&snapshot, "launch")), keys(launch));
        assert_eq!(
            names(find(&snapshot, "terminal")),
            keys(SnapshotTerminal {
                history_lines: 5000
            })
        );
        fn inspect(f: &Field) {
            match &f.kind {
                Kind::Object(fields) => {
                    for child in fields {
                        inspect(child)
                    }
                }
                Kind::List { item, .. } => inspect(item),
                Kind::Toggle { .. } => assert!(f.default.is_bool(), "{}", f.key),
                Kind::Choice(values) => assert!(
                    values
                        .iter()
                        .any(|v| f.default.as_str() == Some(v) || f.default.to_string() == *v),
                    "{} {:?}",
                    f.key,
                    f.default
                ),
                Kind::Number { min, max, .. } => {
                    assert!((*min..=*max).contains(&f.default.as_integer().unwrap()))
                }
                _ => {}
            }
        }
        inspect(&root);
    }
}
