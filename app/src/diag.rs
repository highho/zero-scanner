//! 附加诊断：把系统错误码翻译成用户能照做的话。
//!
//! 同一个错误码在不同场景下的含义不一样，但「打开进程」这一个场景里
//! 只有几个可能的原因。界面和 MCP 必须给同一句话，否则用户在两个入口
//! 看到两种解释，就不知道该信哪个。

use std::sync::{Arc, OnceLock};

/// 一条诊断：短结论 + 下一步怎么做。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    /// 一句话结论，直接给用户看。
    pub summary: String,
    /// 用户可以尝试的动作；空表示没有可做的事。
    pub action: String,
}

impl Diagnosis {
    fn new(summary: impl Into<String>, action: impl Into<String>) -> Self {
        Self { summary: summary.into(), action: action.into() }
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
            "用「以管理员身份重启」再试一次，或换一个权限相同的进程。",
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
        other => Diagnosis::new(
            format!("无法打开进程（系统错误 {other}）。"),
            "把这条错误码贴出来，可以判断是权限还是目标限制。",
        ),
    }
}

/// 读目标进程内存失败时的诊断表。
///
/// 读失败几乎只有三类原因：地址根本没提交（487）、地址落在地图边界外（299，
/// 与「已被调试」同码但场景不同）、以及目标已退出（5/31/87）。
pub fn for_read_memory(code: u32) -> Diagnosis {
    match code {
        0 => Diagnosis::new(
            "这次读取没有拿到完整字节。",
            "地址可能正好跨在页尾；换成页内地址再试，或者先刷新区域列表。",
        ),
        5 => Diagnosis::new(
            "拒绝访问：这块内存不允许当前权限读取。",
            "以管理员身份重启 Zero，或换一个地址。",
        ),
        6 => Diagnosis::new(
            "进程句柄已经失效。",
            "目标可能已经退出，重新选择进程。",
        ),
        87 => Diagnosis::new("进程已经退出。", "刷新进程列表后重新连接。"),
        299 => Diagnosis::new(
            "这个地址没有映射到目标进程的内存里。",
            "多半是偏移算错了，或那块内存已经被释放；回到地址列表核对基址与偏移。",
        ),
        487 => Diagnosis::new(
            "这个地址没有提交（保留页或空洞）。",
            "换一个地址；如果是结构体字段，先确认结构体基址对得上。",
        ),
        998 => Diagnosis::new(
            "地址无效或没有读取权限。",
            "确认进程还活着，并检查地址是否在模块范围内。",
        ),
        other => Diagnosis::new(
            format!("无法读取内存（系统错误 {other}）。"),
            "把这条错误码贴出来，可以判断是地址问题还是权限问题。",
        ),
    }
}

/// 写目标进程内存失败时的诊断表。
///
/// 写比读多一类失败：地址可读但不可写（代码段、只读数据）。这种情况系统给 487，
/// 也可能给 5，两者都要指向「先改保护」而不是「重试一次」。
pub fn for_write_memory(code: u32) -> Diagnosis {
    match code {
        0 => Diagnosis::new(
            "这次连接只有读取权限，写不进去。",
            "重新连接时用管理员身份启动 Zero；只读连接下冻结与补丁都不会生效。",
        ),
        5 => Diagnosis::new(
            "拒绝访问：这块内存不允许写入。",
            "代码段或只读数据需要先改保护；以管理员身份重启后再试。",
        ),
        6 => Diagnosis::new(
            "进程句柄已经失效。",
            "目标可能已经退出，重新选择进程。",
        ),
        87 => Diagnosis::new("进程已经退出。", "刷新进程列表后重新连接。"),
        299 => Diagnosis::new(
            "写不进去：这个地址不在目标进程的内存映射里。",
            "地址列表里的旧条目多半已经失效；重新扫一次拿到新地址再写。",
        ),
        487 => Diagnosis::new(
            "这个地址不可写（未提交、受保护，或只读）。",
            "用「补丁」页写代码段（那里会自动改保护），或换一个数据地址。",
        ),
        998 => Diagnosis::new(
            "地址无效或没有写入权限。",
            "确认地址在当前进程里有效；补丁类写入请走补丁页。",
        ),
        other => Diagnosis::new(
            format!("无法写入内存（系统错误 {other}）。"),
            "把这条错误码贴出来。",
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
            "进程已被其它调试器附加（同一时间只允许一个调试器）。",
            "关掉那个调试器再试。",
        ),
        31 => Diagnosis::new("目标可能刚崩溃或已退出。", "重新连接后再试。"),
        other => Diagnosis::new(
            format!("无法附加调试器（系统错误 {other}）。"),
            "把这条错误码贴出来。",
        ),
    }
}

/// 最后一次诊断：界面与 MCP 都拿这一份，避免同一个失败在两处说法不同。
static LAST: OnceLock<std::sync::Mutex<Option<Diagnosis>>> = OnceLock::new();

fn last_slot() -> &'static std::sync::Mutex<Option<Diagnosis>> {
    LAST.get_or_init(|| std::sync::Mutex::new(None))
}

/// 记下这次诊断，供「为什么连不上」那类界面回看。
pub fn remember(diagnosis: &Diagnosis) {
    if let Ok(mut slot) = last_slot().lock() {
        *slot = Some(diagnosis.clone());
    }
}

pub fn last() -> Option<Diagnosis> {
    last_slot().lock().ok().and_then(|slot| slot.clone())
}

pub fn clear_last() {
    if let Ok(mut slot) = last_slot().lock() {
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_have_actions() {
        for code in [5u32, 87, 299, 31] {
            let d = for_open_process(code);
            assert!(!d.summary.is_empty(), "code {code} needs a summary");
            assert!(!d.action.is_empty(), "code {code} needs an action");
        }
    }

    #[test]
    fn unknown_code_keeps_the_number() {
        let d = for_open_process(1234);
        assert!(d.summary.contains("1234"), "the number must survive: {}", d.summary);
        assert!(!d.action.is_empty());
    }

    #[test]
    fn line_joins_without_trailing_space() {
        let d = Diagnosis::new("出事了。", "重试一次。");
        assert_eq!(d.line(), "出事了。 重试一次。");
        let bare = Diagnosis::new("出事了。", "");
        assert_eq!(bare.line(), "出事了。");
    }

    #[test]
    fn debugger_wording_differs_from_picker() {
        assert_ne!(for_open_process(5).summary, for_debug_attach(5).summary);
    }

    #[test]
    fn read_and_write_failures_say_different_things() {
        // The same code must not be explained the same way: a failed read means
        // "nothing is there", a failed write means "it is there but locked".
        for code in [5u32, 299, 487, 998] {
            let r = for_read_memory(code);
            let w = for_write_memory(code);
            assert!(!r.action.is_empty(), "read code {code} needs an action");
            assert!(!w.action.is_empty(), "write code {code} needs an action");
            assert_ne!(r.summary, w.summary, "code {code} must read differently for read and write");
        }
        assert!(for_write_memory(0).summary.contains("只有读取权限"));
        assert!(for_read_memory(487).summary.contains("没有提交"));
    }
}
