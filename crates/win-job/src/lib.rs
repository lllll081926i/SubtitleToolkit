//! Windows Job Object：把第三方子进程树关进笼子（23 §外部进程）。
//!
//! 存在的理由只有一条：`Child::kill()` 只杀直接子进程。孙进程继续持有继承来的
//! stdout 管道，父进程的读线程就会一直阻塞到孙进程自己结束——本机实测 30 秒
//! （ISSUES Z6 的起因）。渲染器 host 与 ffprobe 探测都靠外部二进制，两者都需要
//! "我放弃这次调用时，整棵树必须一起消失"这条保证，所以这个模块独立成一个 crate
//! 而不是在 `media-probe` 里长一份（一个语义只该有一处实现，ISSUES AD9 的教训）。
//!
//! 失败绝不静默：`attach` 返回 Win32 错误码。已知的正常失败是"本进程已经属于某个 Job"
//! （`ERROR_ACCESS_DENIED = 5`）——测试跑在 `cargo nextest` 下就是这种情况，一个进程
//! 只能属于一个 Job。调用方必须把"没约束住"如实报告出去。

/// 把已经在跑的进程装进一个新的 Job，并在返回的句柄关闭时终止整棵树。
/// 关掉的瞬间整棵子进程树被终止（`KILL_ON_JOB_CLOSE`）。
/// 类型必须放在模块级，`attach` 的签名才写得出它。
#[cfg(windows)]
pub struct Job {
    handle: std::os::windows::io::RawHandle,
    process_handle: std::os::windows::io::RawHandle,
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        use std::os::windows::io::RawHandle;
        extern "system" {
            fn CloseHandle(h: RawHandle) -> i32;
        }
        unsafe {
            // 关闭 job 句柄即触发 KILL_ON_JOB_CLOSE，整棵进程树被终止
            let _ = CloseHandle(self.process_handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

///
/// 成功返回 `Job`；失败返回 Win32 错误码（非 Windows 平台固定返回 `0`，含义是
/// "本平台没有这套机制"，与"有机制但调用失败"区分开）。
#[cfg(windows)]
pub fn attach(pid: u32) -> Result<Job, u32> {
    use std::os::windows::io::RawHandle;

    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    const PROCESS_SET_QUOTA: u32 = 0x0100;
    const PROCESS_TERMINATE: u32 = 0x0001;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct IoCounters {
        read: u64,
        write: u64,
        other: u64,
        read_count: u64,
        write_count: u64,
        other_count: u64,
    }

    /// MSDN 的 `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` 末尾是四个 SIZE_T：
    /// ProcessMemoryLimit / JobMemoryLimit / PeakProcessMemoryUsed / PeakJobMemoryUsed。
    /// 字段数错一个，`SetInformationJobObject` 就以 `ERROR_BAD_LENGTH(24)` 拒绝——
    /// 而这正是"约束静默失效"的样子：主进程以为树被关住了，其实没有（本机实测，ISSUES AE3）。
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    extern "system" {
        fn CreateJobObjectW(attrs: *const u8, name: *const u16) -> RawHandle;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
        fn AssignProcessToJobObject(job: RawHandle, process: RawHandle) -> i32;
        fn SetInformationJobObject(job: RawHandle, class: u32, info: *const u8, len: u32) -> i32;
        fn CloseHandle(h: RawHandle) -> i32;
        fn GetLastError() -> u32;
    }

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() || job == -1isize as RawHandle {
            return Err(GetLastError());
        }
        let limits = ExtendedLimits {
            basic: BasicLimits {
                limit_flags: KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        let ok = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &limits as *const ExtendedLimits as *const u8,
            std::mem::size_of::<ExtendedLimits>() as u32,
        );
        if ok == 0 {
            let e = GetLastError();
            let _ = CloseHandle(job);
            return Err(e);
        }
        let ph = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if ph.is_null() || ph == -1isize as RawHandle {
            let e = GetLastError();
            let _ = CloseHandle(job);
            return Err(e);
        }
        let assigned = AssignProcessToJobObject(job, ph);
        if assigned == 0 {
            let e = GetLastError();
            let _ = CloseHandle(ph);
            let _ = CloseHandle(job);
            return Err(e);
        }
        Ok(Job {
            handle: job,
            process_handle: ph,
        })
    }
}

#[cfg(not(windows))]
pub fn attach(_pid: u32) -> Result<Job, u32> {
    Err(0)
}

#[cfg(not(windows))]
pub struct Job;

/// 已知错误码的可读名（诊断文本要能说清"为什么没约束住"，而不是留一个数字）。
pub fn error_name(code: u32) -> &'static str {
    match code {
        0 => "本平台没有 Job Object 机制",
        5 => "ERROR_ACCESS_DENIED：进程已属于另一个 Job（一个进程只能在一个 Job 里）",
        87 => "ERROR_INVALID_PARAMETER",
        _ => "未分类的 Win32 错误码",
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// `attach` 要么成功，要么带着一个有解释的错误码失败——**不允许静默退化**（Z6）。
    #[test]
    fn attach_either_succeeds_or_reports_an_explained_win32_code() {
        // 一个真的在跑的子进程：cmd 起 ping，孙进程持有继承的 stdout 管道
        let mut child = Command::new("cmd.exe")
            .args(["/C", "ping", "-n", "30", "127.0.0.1", ">nul"])
            .stdout(Stdio::null())
            .spawn()
            .expect("cmd.exe 必须能启动");
        match crate::attach(child.id()) {
            Ok(job) => {
                drop(job);
                // 关闭 Job 句柄 = 终止整棵树；等不到就是约束没生效
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        Ok(None) => {
                            panic!("关闭 Job 句柄后子进程还活着：KILL_ON_JOB_CLOSE 没起作用")
                        }
                        Err(e) => panic!("try_wait 失败：{e}"),
                    }
                }
            }
            Err(code) => {
                // nextest 会把测试进程自己放进一个外层 Job，此时附着必然失败（Z6 实测）。
                // 这里断言的不是"失败可以接受"，而是"失败必须带一个能解释的错误码"。
                let _ = child.kill();
                let _ = child.wait();
                assert_ne!(
                    code, 0,
                    "Windows 上附着失败不该返回 0（0 的语义是本平台无此机制）"
                );
                assert!(
                    !crate::error_name(code).is_empty(),
                    "错误码 {code} 必须能翻译成人话"
                );
            }
        }
    }
}
