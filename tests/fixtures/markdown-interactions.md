# Markdown 阅读交互验收

点击 [宽表格](#宽表格)、[折叠内容中的标题](#隐藏标题)，再按 Alt+Left 返回。
也可以按 T 输入“表格”，Enter 跳转，Esc 取消。

[打开另一份 Markdown](markdown-reading.md) · [打开实现文档](../../docs/18-Markdown-Reading-Interactions.md)
· [打开浏览器](https://example.com)

## 复制代码

拖动底部横向滚动条后，语言与 Copy 按钮仍留在右上方。

```rust,no_run
fn main() {
    let answer = 42;

    println!("This deliberately long code line verifies horizontal scrolling without moving the copy button: {answer}");
}
```

```
没有声明语言。Copy 仍可复制这一整个代码块。
```

## 图片与脚注

![本地图示：点击使用系统应用打开](markdown-diagram.svg)

这段文字有脚注[^reading]。点击标记跳转到定义，再点 ↩ 返回。

## 折叠内容

<details>
<summary>点击展开更多内容</summary>

### 隐藏标题

这是折叠正文。通过大纲或锚点跳到这里时，外层会自动展开。

<details open>
<summary>默认展开的内层</summary>

支持 **强调**、`行内代码` 和公式 $x^2$。

</details>
</details>

## 引用层级

> 第一层引用。
>
> > 第二层引用，缩窄窗口检查续行缩进和竖线。
> >
> > > 第三层引用。
>
> 回到第一层。

## 宽表格

横向查看描述列；向下滚动时检查表头是否固定。可点击右边框轨道，或拖动青色滑块快速定位。

| 名称 | 长描述 | 状态 |
| --- | --- | --- |
| item 01 | A deliberately wide description column for checking horizontal panning while keeping the full text readable. | done |
| item 02 | The header should remain visible while the body of this table is scrolled. | done |
| item 03 | Click the scrollbar track or drag its thumb to move quickly. | done |
| item 04 | Wide table content keeps its column alignment when panned. | done |
| item 05 | Copy source retains the original Markdown table. | done |
| item 06 | Data row six. | done |
| item 07 | Data row seven. | done |
| item 08 | Data row eight. | done |
| item 09 | Data row nine. | done |
| item 10 | Data row ten. | done |
| item 11 | Data row eleven. | done |
| item 12 | Data row twelve. | done |
| item 13 | Data row thirteen. | done |
| item 14 | Data row fourteen. | done |
| item 15 | Data row fifteen. | done |
| item 16 | Data row sixteen. | done |
| item 17 | Data row seventeen. | done |
| item 18 | Data row eighteen. | done |
| item 19 | Data row nineteen. | done |
| item 20 | Data row twenty. | done |
| item 21 | Data row twenty-one. | done |
| item 22 | Data row twenty-two. | done |
| item 23 | Data row twenty-three. | done |
| item 24 | Data row twenty-four. | done |

## 文档尾部

切换到其他文件再回来，检查位置保留；按 M 切换源码，再按 M 返回。

[^reading]: 脚注定义。点击前面的 ↩，或按 Alt+Left 返回引用位置。
