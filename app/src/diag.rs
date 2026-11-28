//! 附加诊断：把系统错误码翻译成用户能照做的话。
//!
//! 同一个错误码在不同场景下的含义不一样，但「打开进程」这一个场景里
//! 只有几个可能的原因。界面和 MCP 必须给同一句话，否则用户在两个入口
//! 看到两种解释，就不知道该信哪个。

/// 一条诊断：短结论 + 下一步怎么做。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    /// 一句话结论，直接给用户看。
    pub summary: String,
    /// 用户可以尝试的动作；空表示没有可做的事。
    pub action: String,
}

impl Diagnosis {
    fn new(summary: &str, action: &str) -> Self {
        Self {
            summary: summary.to_string(),
            action: action.to_string(),
        }
    }

    /// 拼成一行，供日志 / toast / MCP 回复使用。
    pub fn line(&self) -> String {
        if self.action.is_empty() {
            self.summary.clone()
        } else {
            format!("{} {}", self.summary, self.action)
        }
    }
}

/// 打开进程失败时的诊断表。
///
/// 只列真实会遇到的码：其它一律退回「系统错误 N」并建议把码贴给作者，
/// 不编造不存在的原因。
pub fn for_open_process(code: u32) -> Diagnosis {
    match code {
        5 => Diagnosis::new(
            "拒绝访问：目标进程的权限比 Zero 高。",
            "用管理员身份重启 Zero，或换一个权限相同的进程。",
        ),
        87 => Diagnosis::new(
            "进程已经退出。",
            "刷新进程列表后重新选一个。",
        ),
        299 => Diagnosis::new(
            "这个进程已经被别的调试器附加了。",
            "关掉那个调试器（或游戏自带的调试/反作弊组件）再试。",
        ),
        31 => Diagnosis::new(
            "进程可能刚崩溃或正在退出。",
            "等它彻底退出后刷新列表。",
        ),
        _ => Diagnosis::new(
            &format!("无法打开进程（系统错误 {code}）。"),
            "把这条错误码贴给作者，可以判断是权限还是目标限制。",
        ),
    }
}

/// 附加调试器失败时的诊断表。调试器失败码和打开进程不完全一样：
/// 这里最常见的是「已被调试」（299）与「拒绝访问」（5）。
pub fn for_debug_attach(code: u32) -> Diagnosis {
    match code {
        5 => Diagnosis::new(
            "拒绝访问：目标以更高权限运行或受保护。",
            "以管理员身份运行 Zero。",
        ),
        87 => Diagnosis::new("进程已退出或参数无效。", "刷新进程列表后重试。"),
        299 => Diagnosis::new(
            "进程已被其它调试器附加（只允许一个调试器）。",
            "关掉那个调试器再试。",
        ),
        31 => Diagnosis::new("目标可能刚崩溃或已退出。", "重新连接后再试。"),
        _ => Diagnosis::new(
            &format!("无法附加调试器（系统错误 {code}）。"),
            "把这条错误码贴给作者。",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_have_actions() {
        for code in [5u32, 87, 299, 31] {
            let d = for_open_process(code);
            assert!(!d.action.is_empty(), "code {code} 应当给出下一步");
            assert!(!d.summary.contains("系统错误"), "code {code} 不该退回通用文案");
        }
    }

    #[test]
    fn unknown_code_keeps_the_number() {
        let d = for_open_process(1234);
        assert!(d.summary.contains("1234"));
        assert!(d.line().contains("1234"));
    }

    #[test]
    fn line_joins_without_trailing_space() {
        let d = Diagnosis::new("结论。", "");
        assert_eq!(d.line(), "结论。");
    }
}
