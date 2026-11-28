# 主题与主题包

Zero 1.2 起支持运行时主题：界面颜色不再是编译期常量，而是一套可以在设置里一键切换、
也可以用 JSON 主题包扩展的调色板。

## 内置主题

| id | 名称 | 说明 |
| --- | --- | --- |
| midnight | 深空蓝 | 默认主题，适合长时间夜视 |
| neon | 霓虹赛博 | 高对比粉青撞色 |
| esports | 电竞紫 | 紫色电竞氛围 |
| tactical | 战术绿 | 军绿终端感 |
| ember | 暗夜琥珀 | 暖琥珀，低蓝光 |
| ice | 极光冰蓝 | 冷青色 |
| graphite | 石墨灰 | 近乎单色的克制灰 |
| daylight | 晨曦 | 唯一的浅色主题，适合白天 |

## 主题包

主题包是放在主题文件夹里的 JSON 文件：

- Windows：`%APPDATA%/Zero/themes/*.json`
- 也可以在设置 → 外观与主题 → 「打开主题文件夹」直接打开

最小示例（继承某个内置主题，只覆盖部分颜色）：

```json
{
  "id": "candy-pop",
  "name": "糖果汽水",
  "base": "midnight",
  "dark": true,
  "colors": {
    "accent": "#ff6bc1",
    "accent_2": "#4de3ff"
  }
}
```

字段说明：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| id | 是 | 唯一 id；与内置主题重复时自动加 -2 后缀 |
| name | 是 | 显示名 |
| base | 是 | 要继承的内置主题 id（或先加载的另一包 id） |
| dark | 否 | 是否深色（影响 egui 控件风格，默认 true） |
| description | 否 | 一行说明，显示在设置里 |
| colors | 否 | 要覆盖的颜色键值表，`#rrggbb` 或 `#rrggbbaa` |

完整颜色键：`bg` `bg_top` `panel` `sidebar` `card` `card_hi` `card_soft`
`border` `border_soft` `text` `muted` `faint` `accent` `accent_2`
`accent_dark` `warn` `danger` `success` `glow_a` `glow_b` `logo_a`
`logo_b` `on_accent`。

改完点「重新扫描」即生效；解析失败的包不会让程序崩溃，会在设置里列出跳过原因。

仓库里的示例：[`themes/candy-pop.json`](../themes/candy-pop.json)、
[`themes/terminal-amber.json`](../themes/terminal-amber.json)。把这两个文件复制进
主题文件夹即可直接使用。

## 命令行

```
zero-scanner.exe --theme=neon          # 用指定主题启动
zero-scanner.exe --tab=struct --theme=daylight
```

选择的主题会记住在 `%APPDATA%/Zero/prefs.json`，下次启动自动恢复。
