import { useEffect, useMemo, useState, type ReactNode } from "react";
import clsx from "clsx";
import { Bot, CheckCircle2, Copy, KeyRound, Plug, Plus, RefreshCw, RotateCcw, Server, ShieldAlert, Tags, Trash2, Workflow } from "lucide-react";
import { api, errorText, type AppConfig, type NamedId } from "../lib/api";
import { useApp } from "../lib/store";
import { useToast } from "../lib/toast";
import { isMac } from "../lib/format";
import { Badge, Button, Card, Field, Input, Select, Switch, Textarea } from "../components/ui";

const DEFAULT_RULES: AppConfig["rules"] = [
  { category: "缺陷", keywords: ["报错", "错误", "异常", "bug", "崩溃", "失败", "无法", "不能", "闪退", "白屏", "500", "exception"] },
  { category: "权限配置", keywords: ["权限", "账号", "密码", "开通", "角色", "授权", "登录不了", "配置"] },
  { category: "数据处理", keywords: ["数据", "导入", "导出", "修改数据", "删除数据", "统计", "报表", "sql"] },
  { category: "环境问题", keywords: ["服务器", "部署", "网络", "超时", "磁盘", "内存", "宕机", "证书", "环境"] },
  { category: "需求", keywords: ["新增", "增加", "希望", "需求", "优化", "建议", "改进", "支持"] },
  { category: "咨询", keywords: ["如何", "怎么", "请问", "是否", "能否", "咨询"] },
];

const SECTIONS = [
  { id: "redmine", label: "Redmine 连接", icon: Server },
  { id: "sync", label: "同步与提醒", icon: RefreshCw },
  { id: "workflow", label: "工作流", icon: Workflow },
  { id: "ai", label: "AI 模型", icon: Bot },
  { id: "mcp", label: "MCP 服务", icon: Plug },
  { id: "rules", label: "分类规则", icon: Tags },
] as const;

export function SettingsPage() {
  const { config, reloadConfig, secrets, reloadSecrets, mcp, reloadMcp, bump, intent, clearIntent } = useApp();
  const toast = useToast();
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [redmineKey, setRedmineKey] = useState("");
  const [aiKey, setAiKey] = useState("");
  const [testing, setTesting] = useState<"redmine" | "ai" | null>(null);
  const [statuses, setStatuses] = useState<NamedId[]>([]);
  const [activities, setActivities] = useState<NamedId[]>([]);
  const [projects, setProjects] = useState<{ id: number; name: string }[]>([]);
  const [clientTab, setClientTab] = useState<"claude" | "http" | "stdio">("claude");
  const [autostart, setAutostart] = useState<boolean | null>(null);

  useEffect(() => {
    if (config) setDraft(structuredClone(config));
  }, [config]);

  useEffect(() => {
    api.getAutostart().then(setAutostart).catch(() => setAutostart(null));
  }, []);

  // 命令面板跳转到指定小节
  useEffect(() => {
    if (intent?.type !== "settingsSection" || !draft) return;
    const id = intent.id;
    clearIntent();
    window.setTimeout(() => document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" }), 50);
  }, [intent, clearIntent, draft]);

  async function toggleAutostart(v: boolean) {
    try {
      await api.setAutostart(v);
      setAutostart(v);
      toast.show({ tone: "success", title: v ? "已设置开机自动启动" : "已取消开机自动启动" });
    } catch (e) {
      toast.show({ tone: "error", title: "设置开机启动失败", body: errorText(e) });
    }
  }

  useEffect(() => {
    if (!secrets.redmine) return;
    api.statuses().then(setStatuses).catch(() => {});
    api.activities().then(setActivities).catch(() => {});
    api
      .listIssues({ includeDone: true })
      .then((xs) => {
        const m = new Map<number, string>();
        xs.forEach((x) => m.set(x.projectId, x.projectName));
        setProjects([...m.entries()].map(([id, name]) => ({ id, name })));
      })
      .catch(() => {});
  }, [secrets.redmine, config?.redmine.baseUrl]);

  const dirty = useMemo(() => !!draft && !!config && JSON.stringify(draft) !== JSON.stringify(config), [draft, config]);

  if (!draft) return null;

  function set<K extends keyof AppConfig>(section: K, patch: Partial<AppConfig[K]>) {
    setDraft((d) => (d ? { ...d, [section]: { ...(d[section] as object), ...patch } } : d));
  }

  async function save() {
    if (!draft) return;
    setSaving(true);
    try {
      // 输入时保留空项以便继续输入分隔符，保存时再清理
      const clean: AppConfig = {
        ...draft,
        workflow: { ...draft.workflow, noteSections: draft.workflow.noteSections.filter(Boolean) },
        rules: draft.rules.filter((r) => r.category.trim()).map((r) => ({ ...r, category: r.category.trim(), keywords: r.keywords.filter(Boolean) })),
      };
      await api.saveConfig(clean);
      await reloadConfig();
      await reloadMcp();
      bump();
      toast.show({ tone: "success", title: "设置已保存" });
    } catch (e) {
      toast.show({ tone: "error", title: "保存失败", body: errorText(e) });
    } finally {
      setSaving(false);
    }
  }

  async function saveSecret(kind: "redmineApiKey" | "aiApiKey", value: string) {
    try {
      await api.setSecret(kind, value);
      await reloadSecrets();
      kind === "redmineApiKey" ? setRedmineKey("") : setAiKey("");
      toast.show({ tone: "success", title: "密钥已保存到系统钥匙串" });
    } catch (e) {
      toast.show({ tone: "error", title: "保存密钥失败", body: errorText(e) });
    }
  }

  async function testRedmine() {
    if (dirty) await save();
    setTesting("redmine");
    try {
      const u = await api.testRedmine();
      toast.show({ tone: "success", title: "连接成功", body: `当前用户：${u.lastname}${u.firstname}（${u.login}）` });
      api.statuses().then(setStatuses).catch(() => {});
      api.activities().then(setActivities).catch(() => {});
    } catch (e) {
      toast.show({ tone: "error", title: "连接失败", body: errorText(e) });
    } finally {
      setTesting(null);
    }
  }

  async function testAi() {
    if (dirty) await save();
    setTesting("ai");
    try {
      toast.show({ tone: "success", title: "AI 连接成功", body: await api.testAi() });
    } catch (e) {
      toast.show({ tone: "error", title: "AI 连接失败", body: errorText(e) });
    } finally {
      setTesting(null);
    }
  }

  function copy(text: string) {
    void navigator.clipboard.writeText(text).then(() => toast.show({ tone: "success", title: "已复制" }));
  }

  const statusSelect = (value: number | null | undefined, onChange: (v: number | null) => void) => (
    <Select value={value ?? ""} onChange={(e) => onChange(e.target.value ? Number(e.target.value) : null)}>
      <option value="">{statuses.length ? "不设置" : "（先连接 Redmine）"}</option>
      {statuses.map((s) => (
        <option key={s.id} value={s.id}>
          {s.name}
          {s.isClosed ? "（关闭）" : ""}
        </option>
      ))}
    </Select>
  );

  const port = mcp?.port ?? draft.mcp.port;
  const url = `http://127.0.0.1:${port}/mcp`;
  const token = config?.mcp.token ?? "";
  const snippets = {
    claude: `claude mcp add --transport http redpilot ${url} \\\n  --header "Authorization: Bearer ${token}"`,
    http: JSON.stringify({ mcpServers: { redpilot: { url, headers: { Authorization: `Bearer ${token}` } } } }, null, 2),
    stdio: JSON.stringify({ mcpServers: { redpilot: { command: mcp?.exePath || "redpilot", args: ["--mcp-stdio"] } } }, null, 2),
  };

  return (
    <div className="flex h-full min-w-0 flex-col">
      <header data-tauri-drag-region className={clsx("flex shrink-0 items-center gap-3 border-b border-line px-6", isMac ? "h-[52px]" : "h-12")}>
        <h2 data-tauri-drag-region className="text-[15px] font-semibold">设置</h2>
        <div className="flex-1" data-tauri-drag-region />
        {dirty && <span className="text-[12.5px] text-warn">有未保存的修改</span>}
        <Button variant="ghost" disabled={!dirty} onClick={() => config && setDraft(structuredClone(config))}>
          还原
        </Button>
        <Button variant="primary" disabled={!dirty} loading={saving} onClick={() => void save()}>
          保存设置
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <nav className="w-[180px] shrink-0 space-y-0.5 border-r border-line p-3">
          {SECTIONS.map((s) => (
            <a
              key={s.id}
              href={`#${s.id}`}
              className="flex h-8 items-center gap-2 rounded-lg px-2.5 text-[13px] text-ink-2 hover:bg-hover hover:text-ink"
              onClick={(e) => {
                e.preventDefault();
                document.getElementById(s.id)?.scrollIntoView({ behavior: "smooth", block: "start" });
              }}
            >
              <s.icon className="size-4 text-ink-3" />
              {s.label}
            </a>
          ))}
        </nav>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-[760px] space-y-6 p-6 pb-24">
            {/* Redmine */}
            <Section id="redmine" title="Redmine 连接" desc="需要管理员在 Redmine「管理 → 配置 → API」中启用 REST web service。">
              <Field label="服务器地址">
                <Input value={draft.redmine.baseUrl} onChange={(e) => set("redmine", { baseUrl: e.target.value })} placeholder="https://redmine.example.com" />
              </Field>
              <Field
                label={<KeyLabel label="API Key" saved={secrets.redmine} />}
                hint="在 Redmine「我的账号」页面右侧「API 访问键」处查看。密钥只保存在系统钥匙串中。"
              >
                <div className="flex gap-2">
                  <Input type="password" value={redmineKey} onChange={(e) => setRedmineKey(e.target.value)} placeholder={secrets.redmine ? "••••••••（已保存，输入新值可替换）" : "粘贴 API Key"} />
                  <Button disabled={!redmineKey.trim()} onClick={() => void saveSecret("redmineApiKey", redmineKey)}>
                    保存密钥
                  </Button>
                </div>
              </Field>
              <div className="grid grid-cols-2 gap-4">
                <Field label="文本格式" hint="与 Redmine「设置 → 显示 → 文本格式」保持一致">
                  <Select value={draft.redmine.textFormat} onChange={(e) => set("redmine", { textFormat: e.target.value as "markdown" | "textile" })}>
                    <option value="markdown">Markdown / CommonMark</option>
                    <option value="textile">Textile</option>
                  </Select>
                </Field>
                <Field label="代理（可选）" hint="留空使用系统代理（内网地址自动直连）；填 direct 强制直连；同时用于 AI 请求">
                  <Input value={draft.redmine.proxy} onChange={(e) => set("redmine", { proxy: e.target.value })} placeholder="http://127.0.0.1:7890" />
                </Field>
              </div>
              <Toggle label="忽略证书错误" desc="仅在内网使用自签名证书时开启，存在中间人攻击风险" checked={draft.redmine.acceptInvalidCerts} onChange={(v) => set("redmine", { acceptInvalidCerts: v })} />
              <div>
                <Button icon={<CheckCircle2 className="size-3.5" />} loading={testing === "redmine"} disabled={!draft.redmine.baseUrl || !secrets.redmine} onClick={() => void testRedmine()}>
                  测试连接
                </Button>
              </div>
            </Section>

            {/* 同步 */}
            <Section id="sync" title="同步与提醒" desc="RedPilot 在后台按间隔拉取任务，关闭窗口后仍在托盘中运行。">
              {autostart !== null && (
                <Toggle label="开机自动启动" desc="登录系统后在托盘中静默运行，按时同步并提醒；此项立即生效" checked={autostart} onChange={(v) => void toggleAutostart(v)} />
              )}
              <div className="grid grid-cols-2 gap-4">
                <Field label="自动同步间隔">
                  <Select value={draft.sync.intervalMinutes} onChange={(e) => set("sync", { intervalMinutes: Number(e.target.value) })}>
                    <option value={0}>关闭自动同步</option>
                    {[1, 3, 5, 10, 15, 30, 60].map((m) => (
                      <option key={m} value={m}>
                        每 {m} 分钟
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field label="限定项目（可选）" hint="项目标识符或 ID，留空为全部项目">
                  <Input value={draft.sync.project} onChange={(e) => set("sync", { project: e.target.value })} placeholder="例如 erp" />
                </Field>
              </div>
              <Toggle label="指派给我的任务" checked={draft.sync.assignedToMe} onChange={(v) => set("sync", { assignedToMe: v })} />
              <Toggle label="我关注的任务" checked={draft.sync.watched} onChange={(v) => set("sync", { watched: v })} />
              <Field label="额外拉取已保存的查询（可选）" hint="Redmine 中自定义查询的 ID（查询链接里的 query_id）">
                <div className="w-40">
                <Input
                  type="number"
                  value={draft.sync.queryId ?? ""}
                  onChange={(e) => set("sync", { queryId: e.target.value ? Number(e.target.value) : null })}
                  placeholder="例如 12"
                />
                </div>
              </Field>
              <Toggle label="新任务通知" checked={draft.sync.notify} onChange={(v) => set("sync", { notify: v })} />
              <Toggle
                label="重要动态通知"
                desc="提出人回复、任务被转回给你、被调为高优先级时提醒；你自己的操作不会提醒"
                checked={draft.sync.notifyChanges}
                onChange={(v) => set("sync", { notifyChanges: v })}
              />
              <Toggle label="到期提醒" desc="任务今天或明天到期时提醒，每个任务每天最多一次" checked={draft.sync.notifyDue} onChange={(v) => set("sync", { notifyDue: v })} />
              <Toggle label="新任务自动 AI 分析" desc="同步到新任务或任务有更新时，自动调用 AI 做分类与摘要" checked={draft.sync.autoAnalyze} onChange={(v) => set("sync", { autoAnalyze: v })} />
            </Section>

            {/* 工作流 */}
            <Section id="workflow" title="工作流" desc="一键回派时使用的默认状态。各公司工作流不同，请按实际情况选择。">
              <div className="grid grid-cols-3 gap-4">
                <Field label="处理完成后的状态">{statusSelect(draft.workflow.resolvedStatusId, (v) => set("workflow", { resolvedStatusId: v }))}</Field>
                <Field label="开始处理的状态">{statusSelect(draft.workflow.inProgressStatusId, (v) => set("workflow", { inProgressStatusId: v }))}</Field>
                <Field label="追问提出人的状态">{statusSelect(draft.workflow.feedbackStatusId, (v) => set("workflow", { feedbackStatusId: v }))}</Field>
              </div>
              <div className="grid grid-cols-2 gap-4">
                <Field label="工时默认活动类型">
                  <Select value={draft.workflow.defaultActivityId ?? ""} onChange={(e) => set("workflow", { defaultActivityId: e.target.value ? Number(e.target.value) : null })}>
                    <option value="">使用 Redmine 默认</option>
                    {activities.map((a) => (
                      <option key={a.id} value={a.id}>
                        {a.name}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field label="处置说明小节" hint="用顿号或逗号分隔">
                  <Input
                    value={draft.workflow.noteSections.join("、")}
                    onChange={(e) => set("workflow", { noteSections: splitList(e.target.value) })}
                  />
                </Field>
              </div>
              <Toggle label="处理完成时完成度设为 100%" checked={draft.workflow.setDoneRatio} onChange={(v) => set("workflow", { setDoneRatio: v })} />
            </Section>

            {/* AI */}
            <Section id="ai" title="AI 模型" desc="默认对接 DeepSeek，也兼容任何 OpenAI 协议的接口（私有化部署的 DeepSeek、Ollama、vLLM 等）。">
              <Toggle label="启用 AI 功能" checked={draft.ai.enabled} onChange={(v) => set("ai", { enabled: v })} />
              <div className="grid grid-cols-2 gap-4">
                <Field label="接口地址">
                  <Input value={draft.ai.baseUrl} onChange={(e) => set("ai", { baseUrl: e.target.value })} placeholder="https://api.deepseek.com" />
                </Field>
                <Field label={<KeyLabel label="API Key" saved={secrets.ai} />}>
                  <div className="flex gap-2">
                    <Input type="password" value={aiKey} onChange={(e) => setAiKey(e.target.value)} placeholder={secrets.ai ? "••••••••（已保存）" : "sk-..."} />
                    <Button disabled={!aiKey.trim()} onClick={() => void saveSecret("aiApiKey", aiKey)}>
                      保存
                    </Button>
                  </div>
                </Field>
                <Field label="分析模型" hint="日常分类、摘要、处置说明">
                  <Input value={draft.ai.model} onChange={(e) => set("ai", { model: e.target.value })} placeholder="deepseek-chat" />
                </Field>
                <Field label="规划模型（可选）" hint="合并规划用，可填推理模型；留空同分析模型">
                  <Input value={draft.ai.planningModel} onChange={(e) => set("ai", { planningModel: e.target.value })} placeholder="deepseek-reasoner" />
                </Field>
                <Field label="每批分析任务数">
                  <Input type="number" min={1} max={20} value={draft.ai.batchSize} onChange={(e) => set("ai", { batchSize: Number(e.target.value) || 8 })} />
                </Field>
                <Field label="温度">
                  <Input type="number" min={0} max={1.5} step={0.1} value={draft.ai.temperature} onChange={(e) => set("ai", { temperature: Number(e.target.value) })} />
                </Field>
              </div>
              <div className="grid grid-cols-[1fr_1fr_100px] gap-4">
                <Field label="输入单价（每百万 token）" hint="用于在统计页估算费用">
                  <Input type="number" min={0} step={0.1} value={draft.ai.priceInput} onChange={(e) => set("ai", { priceInput: Number(e.target.value) || 0 })} />
                </Field>
                <Field label="输出单价（每百万 token）" hint="默认按 DeepSeek 官网标价，价格调整时请修改">
                  <Input type="number" min={0} step={0.1} value={draft.ai.priceOutput} onChange={(e) => set("ai", { priceOutput: Number(e.target.value) || 0 })} />
                </Field>
                <Field label="货币符号">
                  <Input value={draft.ai.currency} onChange={(e) => set("ai", { currency: e.target.value })} placeholder="¥" />
                </Field>
              </div>
              <Toggle label="JSON 输出模式" desc="部分推理模型或本地模型不支持时请关闭" checked={draft.ai.jsonMode} onChange={(v) => set("ai", { jsonMode: v })} />
              <Toggle label="发送前脱敏" desc="自动遮盖手机号、身份证、邮箱、IP、密码、密钥串等" checked={draft.ai.maskSensitive} onChange={(v) => set("ai", { maskSensitive: v })} />
              {projects.length > 0 && (
                <Field label="不发送给 AI 的项目" hint="涉密项目勾选后，其任务内容不会发送给 AI">
                  <div className="flex flex-wrap gap-2">
                    {projects.map((p) => {
                      const on = draft.ai.excludedProjectIds.includes(p.id);
                      return (
                        <button
                          key={p.id}
                          onClick={(e) => {
                            e.preventDefault();
                            set("ai", { excludedProjectIds: on ? draft.ai.excludedProjectIds.filter((x) => x !== p.id) : [...draft.ai.excludedProjectIds, p.id] });
                          }}
                          className={clsx("rounded-full border px-2.5 py-1 text-[12.5px]", on ? "border-accent bg-accent-soft text-accent-ink" : "border-line text-ink-2 hover:bg-hover")}
                        >
                          {p.name}
                        </button>
                      );
                    })}
                  </div>
                </Field>
              )}
              <details className="group rounded-xl border border-line" open={Object.values(draft.ai.customPrompts).some((v) => v.trim())}>
                <summary className="flex cursor-default list-none items-center justify-between px-3.5 py-2.5">
                  <span>
                    <span className="text-[13px] font-medium text-ink">自定义提示词</span>
                    <span className="ml-2 text-[12px] text-ink-3">追加到各环节的系统提示词后面，例如团队术语、模块划分、说明的语气</span>
                  </span>
                  <span className="text-[12px] text-ink-3 group-open:hidden">展开</span>
                </summary>
                <div className="space-y-3 border-t border-line p-3.5">
                  {(
                    [
                      ["classify", "分类与分析", "例如：模块只能从「订单、库存、财务、权限、报表」中选择；涉及生产数据的任务紧急度至少为“中”"],
                      ["plan", "合并规划", "例如：同一个客户的任务尽量放在同一组；周五下午不安排发布"],
                      ["note", "处置说明", "例如：结尾加上“如有疑问请联系运维组 8008”；不要使用技术术语"],
                      ["report", "日报 / 周报", "例如：按项目分组汇总；最后列出下周计划"],
                    ] as const
                  ).map(([k, label, placeholder]) => (
                    <Field key={k} label={label}>
                      <Textarea
                        rows={2}
                        value={draft.ai.customPrompts[k]}
                        onChange={(e) => set("ai", { customPrompts: { ...draft.ai.customPrompts, [k]: e.target.value } })}
                        placeholder={placeholder}
                      />
                    </Field>
                  ))}
                </div>
              </details>
              <div>
                <Button icon={<CheckCircle2 className="size-3.5" />} loading={testing === "ai"} onClick={() => void testAi()}>
                  测试 AI 连接
                </Button>
              </div>
            </Section>

            {/* MCP */}
            <Section
              id="mcp"
              title="MCP 服务"
              desc="让 Claude、Cursor、Cherry Studio 等 AI 助手直接调用 RedPilot：查询任务、AI 分析、合并规划、起草说明、回派。"
              right={
                mcp?.running ? <Badge tone="ok">运行中 · {mcp.url}</Badge> : mcp?.enabled ? <Badge tone="accent">启动失败</Badge> : <Badge>已关闭</Badge>
              }
            >
              {mcp?.error && <div className="rounded-lg bg-accent-soft p-3 text-[12.5px] text-accent-ink">{mcp.error}</div>}
              <div className="grid grid-cols-[1fr_160px] gap-4">
                <Toggle label="启用本地 HTTP MCP 服务" desc="仅监听 127.0.0.1，需携带访问令牌" checked={draft.mcp.enabled} onChange={(v) => set("mcp", { enabled: v })} />
                <Field label="端口">
                  <Input type="number" value={draft.mcp.port} onChange={(e) => set("mcp", { port: Number(e.target.value) || 39217 })} />
                </Field>
              </div>
              <Field label="访问令牌">
                <div className="flex gap-2">
                  <Input readOnly value={token} className="font-mono text-[12px]" />
                  <Button icon={<Copy className="size-3.5" />} onClick={() => copy(token)}>
                    复制
                  </Button>
                  <Button
                    variant="ghost"
                    icon={<RotateCcw className="size-3.5" />}
                    onClick={() =>
                      void api.regenerateMcpToken().then(async () => {
                        await reloadConfig();
                        await reloadMcp();
                        toast.show({ tone: "success", title: "已生成新令牌", body: "已配置的客户端需要更新令牌" });
                      })
                    }
                  >
                    重新生成
                  </Button>
                </div>
              </Field>
              <div className={clsx("rounded-xl border p-3.5", draft.mcp.allowWrite ? "border-warn/40 bg-warn-soft" : "border-line")}>
                <Toggle
                  label="允许外部 AI 执行写操作"
                  desc="开启后，外部 AI 可以回派任务、追问、添加备注、修改状态、建立关联，这些操作会直接通知 Redmine 上的相关人员。"
                  checked={draft.mcp.allowWrite}
                  onChange={(v) => set("mcp", { allowWrite: v })}
                  icon={<ShieldAlert className="size-4 text-warn" />}
                />
              </div>

              <div>
                <div className="mb-2 flex gap-1 rounded-lg bg-sunken p-1">
                  {(
                    [
                      ["claude", "Claude Code"],
                      ["http", "Cursor / Cherry Studio 等（HTTP）"],
                      ["stdio", "Claude Desktop 等（stdio）"],
                    ] as const
                  ).map(([k, label]) => (
                    <button
                      key={k}
                      onClick={() => setClientTab(k)}
                      className={clsx("flex-1 rounded-md px-2 py-1.5 text-[12.5px]", clientTab === k ? "bg-panel font-medium text-ink shadow-sm" : "text-ink-3 hover:text-ink")}
                    >
                      {label}
                    </button>
                  ))}
                </div>
                <div className="relative">
                  <pre className="selectable overflow-x-auto rounded-xl bg-[#18181b] p-4 font-mono text-[12px] leading-relaxed text-[#e4e4e7]">{snippets[clientTab]}</pre>
                  <button onClick={() => copy(snippets[clientTab])} className="absolute top-2.5 right-2.5 rounded-md bg-white/10 p-1.5 text-white/70 hover:bg-white/20 hover:text-white" title="复制">
                    <Copy className="size-3.5" />
                  </button>
                </div>
                <div className="mt-2 text-[12px] text-ink-3">
                  {clientTab === "stdio"
                    ? "stdio 模式由客户端直接启动 RedPilot，不需要令牌，也不要求桌面端在运行；与桌面端共用配置和钥匙串。"
                    : "HTTP 模式需要 RedPilot 桌面端保持运行（可最小化到托盘）。"}
                </div>
              </div>
            </Section>

            {/* 分类规则 */}
            <Section
              id="rules"
              title="分类规则"
              desc="关键词规则离线运行、不消耗 token，在 AI 分析之前给出初步分类；规则中的分类也是 AI 分类的候选项。"
              right={
                <Button size="sm" variant="ghost" icon={<RotateCcw className="size-3" />} onClick={() => setDraft({ ...draft, rules: structuredClone(DEFAULT_RULES) })}>
                  恢复默认
                </Button>
              }
            >
              <div className="space-y-2">
                {draft.rules.map((r, i) => (
                  <div key={i} className="flex items-center gap-2">
                    <div className="w-28 shrink-0">
                      <Input
                        value={r.category}
                        onChange={(e) => setDraft({ ...draft, rules: draft.rules.map((x, j) => (j === i ? { ...x, category: e.target.value } : x)) })}
                        className="font-medium"
                      />
                    </div>
                    <div className="min-w-0 flex-1">
                    <Input
                      value={r.keywords.join("、")}
                      onChange={(e) =>
                        setDraft({ ...draft, rules: draft.rules.map((x, j) => (j === i ? { ...x, keywords: splitList(e.target.value) } : x)) })
                      }
                      placeholder="关键词，用顿号分隔"
                    />
                    </div>
                    <button className="rounded-md p-1.5 text-ink-3 hover:bg-hover hover:text-accent" title="删除" onClick={() => setDraft({ ...draft, rules: draft.rules.filter((_, j) => j !== i) })}>
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                ))}
                <Button size="sm" variant="ghost" icon={<Plus className="size-3.5" />} onClick={() => setDraft({ ...draft, rules: [...draft.rules, { category: "新分类", keywords: [] }] })}>
                  添加分类
                </Button>
              </div>
            </Section>
          </div>
        </div>
      </div>
    </div>
  );
}

function splitList(v: string): string[] {
  return v.split(/[、,，]/).map((x) => x.trim());
}

function Section({ id, title, desc, right, children }: { id: string; title: string; desc?: string; right?: ReactNode; children: ReactNode }) {
  return (
    <Card className="scroll-mt-6 p-5">
      <div id={id} className="mb-4 flex items-start justify-between gap-4">
        <div>
          <div className="text-[14.5px] font-semibold">{title}</div>
          {desc && <div className="mt-0.5 text-[12.5px] text-ink-3">{desc}</div>}
        </div>
        {right}
      </div>
      <div className="space-y-4">{children}</div>
    </Card>
  );
}

function Toggle({ label, desc, checked, onChange, icon }: { label: string; desc?: string; checked: boolean; onChange: (v: boolean) => void; icon?: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex gap-2">
        {icon && <span className="mt-0.5">{icon}</span>}
        <div>
          <div className="text-[13px] font-medium text-ink">{label}</div>
          {desc && <div className="mt-0.5 text-[12px] leading-snug text-ink-3">{desc}</div>}
        </div>
      </div>
      <Switch checked={checked} onChange={onChange} />
    </div>
  );
}

function KeyLabel({ label, saved }: { label: string; saved: boolean }) {
  return (
    <span className="flex items-center gap-1.5">
      <KeyRound className="size-3.5 text-ink-3" />
      {label}
      {saved ? <Badge tone="ok">已保存</Badge> : <Badge tone="warn">未设置</Badge>}
    </span>
  );
}
