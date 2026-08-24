import { workspaceStore, type ProjectItem } from "$lib/workspace.svelte";

/**
 * 构建工作空间增强系统提示：
 * 注入项目列表 + action 链接协议 + 可执行代码块说明。
 */
export function buildWorkspaceContext(): string {
    if (!workspaceStore.projects.length) return "";
    const projList = workspaceStore.projects
        .slice(0, 20)
        .map((p) => `- ${p.name} (路径: ${p.path})`)
        .join("\n");
    return `

当前工作空间信息：
- 名称: ${workspaceStore.activeName || "未设置"}
- 路径: ${workspaceStore.activePath || "未设置"}
项目列表：
${projList}

当用户想要执行操作时，你可以使用特殊链接来触发命令。格式如下：
[按钮文案](action://命令类型?参数名=参数值)

除了 action 链接，你也可以直接输出 shell 命令代码块，
用户可以通过代码块上的「执行」按钮一键运行。
执行时会弹窗确认，用户允许后才会执行，请放心推荐。

可用命令：
- open_project: 打开项目，参数为 path（项目完整路径）

例如：
1. 使用 action 链接打开项目：
好的，我来为你打开 astra 项目：[🚀 打开项目](action://open_project?path=/Users/workplace/astra)

2. 使用可执行代码块运行命令：
运行构建：
\`\`\`bash
cd /Users/workplace/astra && pnpm build
\`\`\`

注意：
1. 链接文案应该简洁明了，包含 emoji 更醒目
2. path 参数必须是完整的项目路径，从上面的项目列表中获取
3. 一次只生成一个 action 链接，避免生成多个以免用户困惑
`;
}

/**
 * 解析用户消息中的 @ 提及（@项目名），返回匹配到的项目列表。
 * 支持：中文、字母、数字、点、连字符。
 */
export function parseMentions(text: string): ProjectItem[] {
    const mentionRegex = /@([\w\u4e00-\u9fff][\w\u4e00-\u9fff.-]*)/g;
    const mentions: ProjectItem[] = [];
    let match;
    while ((match = mentionRegex.exec(text)) !== null) {
        const name = match[1];
        const project = workspaceStore.projects.find(
            (p) => p.name.toLowerCase() === name.toLowerCase(),
        );
        if (project && !mentions.includes(project)) {
            mentions.push(project);
        }
    }
    return mentions;
}
