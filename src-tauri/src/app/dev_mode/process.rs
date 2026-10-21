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

#[cfg(not(unix))]
pub fn terminate_process_tree(child: &mut Child) {
    let _ = child.kill();
}

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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn kill_processes_on_port(_port: u16) {}

/// 让子进程独立成进程组（Unix），便于后续整组终止
#[cfg(unix)]
pub(crate) fn setup_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn setup_process_group(_cmd: &mut Command) {}
