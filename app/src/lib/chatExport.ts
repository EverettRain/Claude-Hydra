type Chat = { title: string; turns: string[] };

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? value as Record<string, unknown> : {};
}

export function readChats(raw: string): Chat[] {
  const rows: unknown = JSON.parse(raw.replace(/^\uFEFF/, ""));
  if (!Array.isArray(rows)) throw new Error("请选择导出包中的 conversations.json。");
  const chats: Chat[] = [];
  for (const row of rows) {
    const chat = record(row);
    if (!Array.isArray(chat.chat_messages)) continue;
    const turns: string[] = [];
    for (const item of chat.chat_messages) {
      const msg = record(item);
      const blocks = Array.isArray(msg.content) ? msg.content.map(record) : [];
      const body = typeof msg.text === "string" && msg.text.trim() ? msg.text :
        blocks.filter(b => b.type === "text" && typeof b.text === "string").map(b => b.text).join("\n");
      if (!body.trim()) continue;
      const speaker = msg.sender === "human" ? "用户" : msg.sender === "assistant" ? "Claude" : "消息";
      turns.push(`${speaker}\n${body}`);
    }
    if (turns.length) chats.push({ title: typeof chat.name === "string" && chat.name.trim() ? chat.name : "无标题聊天", turns });
  }
  if (!chats.length) throw new Error("文件中没有包含可续聊文本的普通聊天。");
  return chats;
}
