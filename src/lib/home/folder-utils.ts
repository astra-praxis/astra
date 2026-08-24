/** 文件夹相关的展示工具（首页卡片与详情共用于保持头像颜色一致） */

/** 提取路径中的文件夹名 */
export function folderName(path: string): string {
    const parts = path.replace(/\\/g, "/").split("/");
    return parts[parts.length - 1];
}

/** 根据名称哈希生成稳定的图标颜色 */
export function folderColor(name: string): string {
    const colors = [
        "#4fc3f7",
        "#ff7043",
        "#66bb6a",
        "#ab47bc",
        "#ffa726",
        "#26c6da",
        "#ec407a",
        "#7e57c2",
        "#8d6e63",
        "#78909c",
        "#29b6f6",
        "#f06292",
    ];
    let hash = 0;
    for (let i = 0; i < name.length; i++) {
        hash = name.charCodeAt(i) + ((hash << 5) - hash);
    }
    return colors[Math.abs(hash) % colors.length];
}

/** 搜索关键词高亮（返回带 <mark> 的 HTML） */
export function highlight(text: string, query: string): string {
    if (!query.trim()) return text;
    const q = query.trim();
    const escaped = q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const regex = new RegExp(`(${escaped})`, "gi");
    return text.replace(regex, "<mark>$1</mark>");
}
