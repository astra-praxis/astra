/**
 * 全局共享的格式化工具。
 * 注意：不同页面对时间戳的语义不同（秒级/毫秒级），按语义选用对应函数。
 */

function pad2(n: number): string {
    return String(n).padStart(2, "0");
}

// ===== 文件大小 =====

/** 文件大小（2 位小数），用于项目源码大小、node_modules 大小等 */
export function formatBytes(bytes: number): string {
    if (bytes === 0) return "0 B";
    const k = 1024;
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + " " + units[i];
}

/** 下载尺寸显示（0 → '—'，自适应小数位） */
export function formatSize(bytes: number): string {
    if (bytes === 0) return "—";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(
        Math.floor(Math.log(bytes) / Math.log(1024)),
        units.length - 1,
    );
    return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

/** 下载速度（0 → 空字符串） */
export function formatSpeed(bytesPerSec: number): string {
    if (bytesPerSec === 0) return "";
    return `${formatSize(bytesPerSec)}/s`;
}

// ===== 时间 =====

/** 秒级时间戳 → 'YYYY-MM-DD HH:mm' */
export function formatTimestampSec(ts: number): string {
    if (!ts) return "";
    const d = new Date(ts * 1000);
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/** 秒级时间戳 → 'HH:mm:ss' */
export function formatClockSec(ts: number): string {
    const d = new Date(ts * 1000);
    return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

/** 毫秒时间戳 → 'HH:mm:ss' */
export function formatClockMs(ts: number): string {
    const d = new Date(ts);
    return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

/** 秒级时间戳 → 相对时间（刚刚 / N 分钟前 / N 小时前 / M月D日 HH:mm） */
export function formatRelativeSec(ts: number | null): string {
    if (!ts) return "—";
    const d = new Date(ts * 1000);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    if (diff < 60000) return "刚刚";
    if (diff < 3600000) return `${Math.floor(diff / 60000)} 分钟前`;
    if (diff < 86400000) return `${Math.floor(diff / 3600000)} 小时前`;
    return `${d.getMonth() + 1}/${d.getDate()} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

/** 秒级时间戳 → 今天显示 'HH:mm'，否则 'M/D HH:mm' */
export function formatDayTimeSec(ts: number): string {
    if (!ts) return "";
    const d = new Date(ts * 1000);
    const now = new Date();
    const isToday = d.toDateString() === now.toDateString();
    const time = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
    if (isToday) return time;
    return `${d.getMonth() + 1}/${d.getDate()} ${time}`;
}
