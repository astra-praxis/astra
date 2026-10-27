use std::process::{Child, Command};

/// 终止进程树（包括 pnpm/node 等子孙进程）
///
/// 启动时通过 process_group(0) 让每个服务自成进程组，
/// 停止时对整个进程组发送 SIGTERM（优雅退出），等待短暂时间后
/// 再发送 SIGKILL（强制），确保端口被释放。
///
/// 注意：不能以“直接子进程是否退出”作为是否强杀的判断依据，
/// 因为 sh 退出后 node 等子孙进程可能仍存活并占用端口。
#[cfg(unix)]
pub fn terminate_process_tree(child: &mut Child) {
    let pid = child.id() as i32;
    if pid <= 1 {
        let _ = child.kill();
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    // 短暂等待，让进程组优雅退出
    std::thread::sleep(std::time::Duration::from_millis(400));
    // 强制结束整个进程组（若组已不存在，kill 返回 ESRCH，无副作用）
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.wait();
}

// Windows 版本见文件下方（taskkill /T 杀整棵进程树）

/// 按端口强制结束占用进程（兜底措施）
///
/// 当进程组终止后端口仍被占用时（如脱离进程组的遗留 node 进程），
/// 通过 lsof 找到监听该端口的进程并强杀，确保端口释放。
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn kill_processes_on_port(port: u16) {
    let port_spec = format!("TCP:{}", port);
    let out = std::process::Command::new("lsof")
        .args(["-t", "-i", &port_spec, "-sTCP:LISTEN"])
        .output();
    if let Ok(out) = out {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Ok(pid) = line.trim().parse::<i32>() {
                if pid > 1 {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn kill_processes_on_port(_port: u16) {}

/// 终止进程树（Windows）：taskkill /T /F 连同子孙进程（npm→node 等）一起杀；
/// child.kill() 只杀直接子进程，会漏掉脱离的孙进程。CREATE_NO_WINDOW
/// 避免闪现控制台窗口。
#[cfg(windows)]
pub fn terminate_process_tree(child: &mut Child) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let pid = child.id();
    if pid > 1 {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    let _ = child.wait();
}

/// 按端口强制结束占用进程（Windows 兜底）：netstat -ano 找出 LISTENING
/// 状态占用该端口的 pid，taskkill 整树强杀。
#[cfg(windows)]
pub fn kill_processes_on_port(port: u16) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = Command::new("netstat")
        .args(["-ano", "-p", "TCP"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // 形如：TCP    0.0.0.0:8787    0.0.0.0:0    LISTENING    1234
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5
            && cols[0].eq_ignore_ascii_case("TCP")
            && cols[3].eq_ignore_ascii_case("LISTENING")
        {
            let local = cols[1];
            if local
                .rsplit_once(':')
                .map(|(_, p)| p == port.to_string())
                .unwrap_or(false)
            {
                if let Ok(pid) = cols[4].parse::<u32>() {
                    if pid > 1 {
                        let _ = Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/T", "/F"])
                            .creation_flags(CREATE_NO_WINDOW)
                            .output();
                    }
                }
            }
        }
    }
}

/// 让子进程独立成进程组（Unix），便于后续整组终止
#[cfg(unix)]
pub(crate) fn setup_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn setup_process_group(_cmd: &mut Command) {}
