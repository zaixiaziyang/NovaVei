    (function () {
      const workbench = document.getElementById("workbench");
      const toastEl = document.getElementById("toast");
      const chatTitle = document.getElementById("chatTitle");
      const overlays = {
        skills: document.getElementById("overlaySkills"),
        mcp: document.getElementById("overlayMcp"),
        settings: document.getElementById("overlaySettings"),
        council: document.getElementById("overlayCouncil"),
      };

      const I18N = {
        zh: {
          "common.close": "关闭",
          "common.save": "保存",
          "shell.settings": "设置",
          "session.ctxAria": "对话操作",
          "session.rename": "重命名",
          "session.renameDialogTitle": "重命名对话",
          "session.renameDialogDescription": "为这个对话设置一个便于识别的标题。",
          "session.renameLabel": "对话标题",
          "session.renameHint": "最多 200 个字符。",
          "session.renameCancel": "取消",
          "session.renameSave": "保存",
          "session.renameSaving": "正在保存…",
          "session.renameEmpty": "请输入对话标题。",
          "session.renameTooLong": "对话标题最多 200 个字符。",
          "session.renameBusy": "该对话正在保存重命名。",
          "session.renamed": "对话已重命名。",
          "session.copy": "复制对话",
          "session.delete": "删除",
          "session.copied": "已复制对话",
          "session.deleted": "已删除对话",
          "session.deleteActive": "已删除当前对话",
          "session.pin": "置顶会话",
          "session.unpin": "取消置顶",
          "session.pinned": "已置顶会话",
          "session.unpinned": "已取消置顶",
          "session.archive": "归档",
          "session.unarchive": "取消归档",
          "session.archived": "已归档会话",
          "session.unarchived": "已取消归档",
          "session.groupPinned": "置顶",
          "session.groupArchived": "归档",
          "project.ctxAria": "项目操作",
          "project.copyPath": "复制路径",
          "project.reveal": "在资源管理器中打开当前项目",
          "project.revealed": "已在资源管理器中打开当前项目",
          "project.revealFailed": "无法在资源管理器中打开当前项目，请检查路径后重试。",
          "project.revealDesktopOnly": "请在 NovaVei 桌面应用中打开项目目录。",
          "project.noCurrent": "当前没有可打开的项目目录。",
          "project.revealRequiresRegistration": "请先将当前历史工作空间添加为项目，再在资源管理器中打开目录。",
          "project.remove": "移除项目文件夹",
          "project.pathCopied": "已复制项目路径",
          "project.removed": "已移除项目文件夹",
          "project.removedCurrent": "已移除当前项目",
          "floor.navAria": "楼层导航",
          "floor.pinned": "收藏",
          "floor.pin": "收藏",
          "floor.unpin": "取消收藏",
          "dock.run": "运行",
          "dock.files": "文件",
          "dock.compare": "对比",
          "dock.labels.run": "运行详情",
          "dock.labels.files": "文件树",
          "dock.labels.git": "Git",
          "dock.labels.compare": "模型对比",
          "dock.toggle.open": "打开工具面板",
          "dock.toggle.close": "关闭工具面板",
          "dock.add": "添加工具面板",
          "dock.remove": "移除当前工具面板",
          "dock.menu.title": "添加到侧边栏",
          "dock.menu.files": "文件",
          "dock.menu.filesHint": "浏览当前项目中的资源",
          "dock.menu.run": "侧边任务",
          "dock.menu.runHint": "查看执行进度、影响与待确认项",
          "dock.menu.git": "Git 审阅",
          "dock.menu.gitHint": "按需查看工作区状态与改动",
          "dock.menu.compare": "模型对比",
          "dock.menu.compareHint": "需要时发起多模型只读对比",
          "dock.menu.browser": "浏览器",
          "dock.menu.browserHint": "在侧栏中打开受控网页",
          "dock.browser": "浏览器",
          "dock.browser.open": "打开",
          "dock.browser.ready": "在地址栏输入公开网页地址；登录和密码输入仅由你本人完成。",
          "dock.browser.empty": "网页将在此侧栏中打开。",
          "dock.empty.title": "按需添加工具",
          "dock.empty.hint": "文件与任务不会常驻占用空间；需要时再打开即可。",
          "dock.empty.action": "添加工具",
          "dock.removed": "已移除「{name}」",
          "settings.title": "设置",
          "settings.navAria": "设置分区",
          "settings.nav.providers": "供应商",
          "settings.nav.system": "系统",
          "settings.nav.tools": "工具",
          "settings.tools.tabsAria": "工具类型",
          "settings.tools.tabMcp": "MCP",
          "settings.tools.tabSkills": "Skills",
          "settings.nav.cron": "定时任务",
          "settings.nav.memory": "记忆",
          "settings.nav.archived": "已归档",
          "settings.nav.council": "专家",
          "settings.nav.about": "关于",
          "settings.archived.title": "已归档对话",
          "settings.archived.hint": "归档对话不会出现在侧边栏。可在此打开、恢复或永久删除。",
          "settings.archived.count": "{count} 个归档对话",
          "settings.archived.empty": "暂无已归档对话。",
          "settings.archived.listAria": "已归档对话列表",
          "settings.archived.deleteWarning": "删除会永久移除对话记录，且不可恢复。",
          "settings.archived.open": "打开",
          "settings.archived.restore": "恢复归档",
          "settings.archived.delete": "删除",
          "settings.archived.unavailableDate": "归档时间未知",
          "settings.archived.unknownProject": "未关联项目",
          "settings.archived.opened": "已打开归档对话",
          "settings.archived.restored": "已恢复归档",
          "settings.archived.deleted": "已删除归档对话",
          "settings.archived.deleteTitle": "删除归档对话",
          "settings.archived.deleteMessage": "确定永久删除会话“{title}”？此操作不可撤销。",
          "settings.archived.busy": "该对话正在处理中。",
          "settings.providers.hint": "管理 API 供应商。第三方同步只读取你手动选择的导出 JSON；不会扫描应用或隐私目录，密钥不会预览或回传到界面。",
          "settings.providers.add": "添加供应商",
          "settings.providers.refresh": "刷新",
          "settings.providers.sync": "从导出 JSON 导入",
          "settings.providers.importTitle": "导入预览",
          "settings.providers.importNote": "已选择的 JSON 仅在 native 中解析；密钥和自定义 Header 不会显示，也不会通过此导入写入。仅接受公开 API 根路径；自定义路径请在编辑器中手动配置。请明确选择要合并的配置。",
          "settings.providers.importSkipped": "已跳过 {count} 条不兼容或重复配置。",
          "settings.providers.importCredential": "检测到导出凭据；它不会显示或写入此导入。",
          "settings.providers.importApiRoot": "API 根路径：{path}",
          "settings.providers.importCredentialReentry": "更新后需重新输入本机凭据：导入目标的 API 根路径或协议已变更。",
          "settings.providers.importAdd": "新增",
          "settings.providers.importUpdate": "更新",
          "settings.providers.importSkip": "跳过",
          "settings.providers.importConflict": "已有本机配置，默认跳过；勾选后才会合并非敏感字段。",
          "settings.providers.importNew": "新配置，勾选后才会添加。",
          "settings.providers.importModels": "{count} 个模型",
          "settings.providers.importCancel": "取消导入",
          "settings.providers.importApply": "合并已选（{count}）",
          "settings.providers.importNone": "请至少选择一项配置。",
          "settings.providers.importConfirm": "将合并 {count} 项已选配置。仅新增或更新列表中勾选的项；不会导入密钥，也不会自动测试网络。继续吗？",
          "settings.providers.importApplied": "已新增 {added} 项、更新 {updated} 项供应商配置。",
          "settings.providers.importCancelled": "已取消选择导出 JSON。",
          "settings.providers.importFailed": "读取导出失败：{error}",
          "settings.providers.listAria": "供应商列表",
          "settings.providers.openaiCompat": "OpenAI 兼容",
          "settings.providers.test": "测试模型",
          "settings.providers.edit": "编辑供应商",
          "settings.providers.duplicate": "复制供应商",
          "settings.providers.duplicated": "已创建「{name}」的副本草稿；请填写 API Key 后保存。",
          "settings.providers.copySuffix": "（副本）",
          "settings.providers.models": "模型",
          "settings.providers.defaultBadge": "默认",
          "settings.providers.statusOk": "已连接",
          "settings.providers.statusConfigured": "已配置",
          "settings.providers.statusTesting": "测试中",
          "settings.providers.statusFailed": "测试失败",
          "settings.providers.statusLocal": "本地",
          "settings.providers.statusNeedKey": "待配置 Key",
          "settings.providers.statusDraft": "未保存",
          "settings.providers.newName": "新供应商",
          "settings.providers.newMeta": "https://api.example.com/v1 · 自定义协议",
          "settings.providers.loading": "正在读取本机供应商设置…",
          "settings.providers.empty": "尚未配置供应商。点击“添加供应商”开始。",
          "settings.providers.unavailable": "桌面设置不可用：请在 NovaVei 桌面应用中打开。",
          "settings.providers.loadFailed": "读取供应商设置失败：{error}",
          "settings.providers.save": "保存配置",
          "settings.providers.cancel": "取消",
          "settings.providers.remove": "删除供应商",
          "settings.providers.name": "名称",
          "settings.providers.id": "标识",
          "settings.providers.type": "协议类型",
          "settings.providers.baseUrl": "Base URL",
          "settings.providers.modelIds": "模型 ID（每行一个）",
          "settings.providers.fetchModels": "获取模型",
          "settings.providers.fetchModelsLoading": "正在从已保存的供应商获取模型…",
          "settings.providers.fetchModelsSuccess": "已获取 {count} 个模型，已写入列表；请审核后保存。",
          "settings.providers.fetchModelsTruncated": "上游结果已截断，仅保留 {count} 个模型；请审核后保存。",
          "settings.providers.fetchModelsEmpty": "供应商未返回可用模型，现有草稿未改动。",
          "settings.providers.fetchModelsUnsupported": "该供应商未提供兼容的模型列表接口，现有草稿未改动。",
          "settings.providers.fetchModelsSavedOnly": "连接设置或 API Key 已修改；请先保存后再获取模型。",
          "settings.providers.fetchModelsUnavailable": "本地获取模型命令不可用：请重启 NovaVei 后重试。",
          "settings.providers.fetchModelsFailed": "获取模型失败：{error}",
          "settings.providers.apiKey": "API Key（留空保留已保存 Key）",
          "settings.providers.requestFormat": "OpenAI 请求格式",
          "settings.providers.default": "设为默认供应商",
          "settings.providers.systemProxy": "通过系统代理请求",
          "settings.providers.editorTitle": "编辑供应商",
          "settings.providers.backToList": "关闭",
          "settings.providers.tabGeneral": "普通配置",
          "settings.providers.tabRequest": "请求配置",
          "settings.providers.basicInfo": "基本信息",
          "settings.providers.enabled": "启用供应商",
          "settings.providers.reentryKey": "连接地址或认证方式已改变，请重新输入 API Key 后保存。",
          "settings.providers.modelCatalogueCheck": "检查连接与模型目录",
          "settings.providers.modelCatalogueHelp": "该检查仅探测模型目录接口，不会发送可计费的补全请求。",
          "settings.providers.modelList": "模型列表",
          "settings.providers.modelSearch": "搜索模型…",
          "settings.providers.modelManual": "手动添加模型 ID",
          "settings.providers.addModel": "添加",
          "settings.providers.requestOptions": "请求选项",
          "settings.providers.promptCaching": "启用 Prompt 缓存",
          "settings.providers.promptCachingHelp": "为支持的接口启用前缀缓存；不保证所有上游都计费优惠。",
          "settings.providers.customHeaders": "自定义请求头",
          "settings.providers.addHeader": "添加请求头",
          "settings.providers.headerNote": "已保存的请求头值不会回显；可替换或清除。禁止 Authorization、Cookie 等保留头。",
          "settings.providers.headerName": "名称",
          "settings.providers.headerValue": "值",
          "settings.providers.headerConfigured": "已配置",
          "settings.providers.headerClear": "清除",
          "settings.providers.headerRemove": "删除",
          "settings.providers.defaultModel": "默认",
          "settings.providers.noModels": "暂无模型。可手动添加，或检查连接后获取模型目录。",
          "settings.providers.draftPrepareFailed": "无法准备连接草稿：{error}",
          "settings.providers.catalogueCheckOk": "模型目录可用 · {detail}",
          "settings.providers.catalogueCheckFailed": "模型目录检查失败：{error}",
          "settings.providers.keyNote": "Key 不会回传到界面；留空保存时沿用 native 中已保存的值。",
          "settings.providers.typeCodex": "OpenAI / Codex",
          "settings.providers.typeClaude": "Anthropic Claude",
          "settings.providers.typeGemini": "Google Gemini",
          "settings.providers.formatResponses": "OpenAI Responses",
          "settings.providers.formatCompletions": "OpenAI Chat Completions",
          "settings.providers.testUnavailable": "本地测试命令不可用：请重启 NovaVei 后重试。",
          "settings.providers.notConfigured": "「{name}」尚未配置 API Key。",
          "settings.providers.testOk": "「{name}」可用 · {detail}",
          "settings.providers.testFailed": "「{name}」测试失败：{error}",
          "settings.providers.saved": "已保存供应商「{name}」",
          "settings.providers.removed": "已删除供应商「{name}」",
          "settings.providers.invalid": "请填写有效的标识、名称、Base URL 和至少一个模型 ID。",
          "settings.system.workdir": "工作目录策略",
          "settings.system.workdirProject": "限制在项目根",
          "settings.system.workdirHelp": "文件、终端与工作区能力始终只绑定当前会话的项目根。额外路径尚未启用；请通过原生文件夹选择器作为新的独立项目打开。",
          "settings.system.secondaryLaunch": "再次启动 NovaVei",
          "settings.system.secondaryLaunchFocus": "显示已打开的窗口",
          "settings.system.secondaryLaunchNewWindow": "打开新窗口",
          "settings.system.secondaryLaunchHelp": "默认显示已打开的 NovaVei。选择打开新窗口时，不会启动第二个后台进程；窗口共享本地数据与服务。",
          "settings.system.historyPageSize": "历史消息分页大小",
          "settings.system.historyPageSizeHelp": "打开会话时每次加载的消息条数；更大的值会增加首屏开销。",
          "settings.system.globalSystemPrompt": "全局提示词",
          "settings.system.globalSystemPromptHelp": "会追加到普通聊天、模型对比和 Council 的原有系统指令中；会占用模型上下文。",
          "settings.system.globalSystemPromptWarning": "请不要填写 API Key、密码或其他凭据。",
          "settings.system.theme": "主题",
          "settings.system.themeSystem": "跟随系统",
          "settings.system.themeLight": "浅色",
          "settings.system.themeDark": "深色",
          "settings.system.uiScale": "界面大小",
          "settings.system.showShortcutHints": "显示界面快捷键文字",
          "settings.system.showFullMessageTimestamp": "显示完整消息日期",
          "settings.system.userColor": "用户消息颜色",
          "settings.system.userColorCustom": "自定义",
          "settings.system.language": "语言",
          "settings.system.languageHint": "切换后设置面板与主界面文案会同步更新。",
          "settings.mcp.runtime": "MCP 运行时",
          "settings.mcp.hint": "MCP 配置与连接能力尚未接入。",
          "settings.mcp.openHub": "打开 MCP 中心",
          "settings.skills.root": "Skills 根目录",
          "settings.skills.hint": "Skills 发现与安装能力尚未接入。",
          "settings.skills.openHub": "打开 Skills 中心",
          "settings.cron.title": "定时任务",
          "settings.cron.hint": "定时任务运行时尚未接入，当前仅保留界面位置。",
          "settings.cron.jobMeta": "尚未加载任务",
          "settings.cron.enabled": "启用",
          "settings.memory.title": "记忆索引",
          "settings.memory.hint": "Markdown 事实源 + SQLite 全文检索。支持全局 / 项目 / 每日。",
          "settings.memory.entries": "条目",
          "settings.memory.today": "今日",
          "settings.memory.quota": "配额",
          "settings.council.title": "专家模板库",
          "settings.council.hint": "Council 运行时尚未接入，当前仅保留配置界面位置。",
          "settings.council.arch": "架构主席",
          "settings.council.archMeta": "综合决议 · 只读",
          "settings.council.security": "安全顾问",
          "settings.council.securityMeta": "威胁建模 · 只读",
          "settings.council.growth": "增长顾问",
          "settings.council.growthMeta": "产品取舍 · 只读",
          "settings.council.builtin": "内置",
          "settings.council.optional": "可选",
          "settings.about.blurb": "Tauri 2 + TypeScript + Rust + embedded Pi",
          "settings.about.skin": "界面方案：Luminous Quiet · 设计 plan4",
          "settings.toast.langZh": "已切换为简体中文",
          "settings.toast.langEn": "Switched to English",
          "theme.toLight": "切换到浅色主题",
          "theme.toDark": "切换到深色主题",
          "theme.switched": "已切换至",
          "settings.system.tabsAria": "系统设置分组",
          "settings.system.tabAppearance": "外观",
          "settings.system.tabBehavior": "行为",
          "settings.system.tabPortable": "便携",
          "settings.portable.title": "运行模式",
          "settings.portable.description": "选择应用在下次启动时使用本机数据目录，或使用 EXE 同级的便携数据目录。",
          "settings.portable.optionsAria": "选择运行模式",
          "settings.portable.installedTitle": "本机版",
          "settings.portable.installedDescription": "数据保存在当前 Windows 用户的应用数据目录中。",
          "settings.portable.portableTitle": "便携版",
          "settings.portable.portableDescription": "数据保存在 EXE 同级的 novavei 文件夹中，并在启动时受密码保护。",
          "settings.portable.isolation": "切换不会移动、复制或删除现有对话、设置和凭据；两种模式的数据保持隔离。",
          "settings.portable.apply": "切换运行模式",
          "settings.system.uiFont": "界面字体",
          "settings.system.codeFont": "代码字体",
          "settings.system.fontSystem": "系统默认",
          "settings.nav.shortcuts": "快捷键",
          "settings.shortcuts.title": "键盘快捷键",
          "settings.shortcuts.hint": "主页会显示常用快捷键提示；可在此关闭这些文字，快捷键本身仍然可用。",
          "settings.shortcuts.search": "搜索对话和项目",
          "settings.shortcuts.findConversation": "在当前会话中查找",
          "settings.shortcuts.newChat": "新建对话",
          "settings.shortcuts.settings": "打开设置",
          "settings.shortcuts.theme": "切换浅色 / 深色",
          "settings.shortcuts.dock": "显示 / 隐藏右侧 dock",
          "settings.shortcuts.conversationOnly": "仅显示对话",
          "settings.shortcuts.escape": "关闭浮层 / 搜索 / 弹层",
          "shortcut.conversationOnly.entered": "已进入仅对话模式",
          "shortcut.conversationOnly.exited": "已退出仅对话模式",
          "settings.memory.tabsAria": "记忆类型",
          "settings.memory.tabProject": "项目记忆",
          "settings.memory.tabLongterm": "长期记忆",
          "settings.memory.tabKnowledge": "知识库",
          "settings.memory.tabUsage": "使用情况",
          "settings.memory.knowledgeTitle": "知识库",
          "settings.memory.knowledgeHint": "知识库将在本机运行时可用时加载。",
          "settings.memory.projectTitle": "项目记忆",
          "settings.memory.projectHint": "项目记忆运行时尚未接入。",
          "settings.memory.projectRoot": "当前项目",
          "settings.memory.projectRootHint": "当前未从运行时加载项目记忆路径。",
          "settings.memory.longTitle": "长期记忆",
          "settings.memory.longHint": "长期记忆运行时尚未接入。",
          "settings.memory.longPath": "存储位置",
          "settings.memory.organize": "整理记忆",
          "settings.memory.export": "导出",
          "settings.memory.clear": "清空",
          "settings.memory.usageTitle": "使用情况",
          "settings.memory.usageHint": "记忆统计运行时尚未接入，当前没有可展示的数据。",
          "settings.memory.usageTotal": "总条目",
          "settings.memory.usageProject": "项目",
          "settings.memory.usageLong": "长期",
          "settings.memory.usageStorage": "磁盘占用",
          "settings.memory.usageHits": "本周检索",
          "settings.memory.usageWrites": "本周写入",
          "settings.memory.usageQuotaLabel": "配额使用",
          "settings.memory.usageQuotaHint": "暂无配额数据",
          "settings.memory.usageRefresh": "刷新统计",
          "settings.memory.usageReport": "导出报告",
          "${statusKey}": "${statusKey}",
          "settings.cron.paused": "暂停",
          "settings.cron.refresh": "刷新",
          "settings.cron.create": "新建任务",
          "settings.cron.statTotal": "全部任务",
          "settings.cron.statEnabled": "已启用",
          "settings.cron.statPaused": "已暂停",
          "settings.cron.runNow": "立即运行",
          "settings.cron.edit": "编辑",
          "settings.cron.logs": "日志",
          "settings.cron.resume": "启用",
          "settings.cron.job1Desc": "运行时尚未加载任务",
          "settings.cron.job2Desc": "运行时尚未加载任务",
          "settings.cron.job3Desc": "运行时尚未加载任务",
          "settings.cron.lastOk": "无运行记录",
          "settings.cron.lastOkWeek": "无运行记录",
          "settings.cron.lastSkip": "无运行记录",
          "settings.council.tabsAria": "专家设置",
          "settings.council.tabExperts": "专家",
          "settings.council.tabCharacters": "角色",
          "settings.council.tabTeams": "专家团",
          "settings.council.tabPrompts": "自定义提示词",
          "settings.council.expertsTitle": "专家",
          "settings.council.expertsHint": "专家配置尚未从 Council 运行时加载。",
          "settings.council.addExpert": "添加专家",
          "settings.council.charactersTitle": "角色",
          "settings.council.charactersHint": "角色配置尚未从 Council 运行时加载。",
          "settings.council.addCharacter": "创建角色",
          "settings.council.charactersLoading": "角色配置尚未加载",
          "settings.council.charactersLoadingHint": "等待本机角色存储接入",
          "settings.council.unavailable": "不可用",
          "settings.council.eng": "工程顾问",
          "settings.council.engMeta": "实现路径 · 可写建议",
          "settings.council.teamsTitle": "专家团",
          "settings.council.teamsHint": "专家团配置尚未从 Council 运行时加载。",
          "settings.council.addTeam": "创建专家团",
          "settings.council.team1": "架构评审团",
          "settings.council.team1Meta": "架构主席 · 安全顾问 · 工程顾问 · 产品顾问",
          "settings.council.team2": "发布把关团",
          "settings.council.team2Meta": "安全顾问 · 工程顾问 · 增长顾问",
          "settings.council.team3": "产品探索团",
          "settings.council.team3Meta": "增长顾问 · 产品顾问 · 架构主席",
          "settings.council.promptsTitle": "自定义提示词",
          "settings.council.promptsHint": "提示词存储尚未从 Council 运行时加载。",
          "settings.council.addPrompt": "新建提示词",
          "settings.council.prompt1": "架构评审·严格模式",
          "settings.council.prompt2": "安全评审清单",
          "settings.council.override": "覆盖",
          "settings.council.custom": "自定义",
          "settings.council.edit": "编辑",
          "settings.council.copy": "复制"
        },
        en: {
          "common.close": "Close",
          "common.save": "Save",
          "shell.settings": "Settings",
          "session.ctxAria": "Conversation actions",
          "session.rename": "Rename",
          "session.renameDialogTitle": "Rename conversation",
          "session.renameDialogDescription": "Give this conversation a title that is easy to recognize.",
          "session.renameLabel": "Conversation title",
          "session.renameHint": "Up to 200 characters.",
          "session.renameCancel": "Cancel",
          "session.renameSave": "Save",
          "session.renameSaving": "Saving…",
          "session.renameEmpty": "Enter a conversation title.",
          "session.renameTooLong": "Conversation titles can be at most 200 characters.",
          "session.renameBusy": "This conversation is already being renamed.",
          "session.renamed": "Conversation renamed.",
          "session.copy": "Copy conversation",
          "session.delete": "Delete",
          "session.copied": "Conversation copied",
          "session.deleted": "Conversation deleted",
          "session.deleteActive": "Current conversation deleted",
          "session.pin": "Pin conversation",
          "session.unpin": "Unpin conversation",
          "session.pinned": "Conversation pinned",
          "session.unpinned": "Conversation unpinned",
          "session.archive": "Archive",
          "session.unarchive": "Unarchive",
          "session.archived": "Conversation archived",
          "session.unarchived": "Conversation unarchived",
          "session.groupPinned": "Pinned",
          "session.groupArchived": "Archived",
          "project.ctxAria": "Project actions",
          "project.copyPath": "Copy path",
          "project.reveal": "Open current project in File Explorer",
          "project.revealed": "Opened the current project in File Explorer",
          "project.revealFailed": "Could not open the current project in File Explorer. Check its path and try again.",
          "project.revealDesktopOnly": "Open the project directory in the NovaVei desktop app.",
          "project.noCurrent": "There is no current project directory to open.",
          "project.revealRequiresRegistration": "Add this historical workspace as a project before opening it in File Explorer.",
          "project.remove": "Remove project folder",
          "project.pathCopied": "Project path copied",
          "project.removed": "Project folder removed",
          "project.removedCurrent": "Current project removed",
          "floor.navAria": "Message navigation",
          "floor.pinned": "Pinned",
          "floor.pin": "Pin",
          "floor.unpin": "Unpin",
          "dock.run": "Run",
          "dock.files": "Files",
          "dock.compare": "Compare",
          "dock.labels.run": "Run details",
          "dock.labels.files": "File tree",
          "dock.labels.git": "Git",
          "dock.labels.compare": "Model compare",
          "dock.toggle.open": "Open tools panel",
          "dock.toggle.close": "Close tools panel",
          "dock.add": "Add tool panel",
          "dock.remove": "Remove current tool panel",
          "dock.menu.title": "Add to sidebar",
          "dock.menu.files": "Files",
          "dock.menu.filesHint": "Browse resources in the current project",
          "dock.menu.run": "Side tasks",
          "dock.menu.runHint": "View progress, impact, and requests for confirmation",
          "dock.menu.git": "Git review",
          "dock.menu.gitHint": "Inspect workspace state and changes when needed",
          "dock.menu.compare": "Model compare",
          "dock.menu.compareHint": "Start a read-only multi-model comparison when needed",
          "dock.menu.browser": "Browser",
          "dock.menu.browserHint": "Open a controlled webpage in the sidebar",
          "dock.browser": "Browser",
          "dock.browser.open": "Open",
          "dock.browser.ready": "Enter a public webpage address; you complete sign-in and password entry yourself.",
          "dock.browser.empty": "A webpage will open in this sidebar.",
          "dock.empty.title": "Add tools when needed",
          "dock.empty.hint": "Files and tasks do not take permanent space. Open one when you need it.",
          "dock.empty.action": "Add a tool",
          "dock.removed": "Removed “{name}”",
          "settings.title": "Settings",
          "settings.navAria": "Settings sections",
          "settings.nav.providers": "Providers",
          "settings.nav.system": "System",
          "settings.nav.tools": "Tools",
          "settings.tools.tabsAria": "Tool types",
          "settings.tools.tabMcp": "MCP",
          "settings.tools.tabSkills": "Skills",
          "settings.nav.cron": "Cron",
          "settings.nav.memory": "Memory",
          "settings.nav.archived": "Archived",
          "settings.nav.council": "Experts",
          "settings.nav.about": "About",
          "settings.archived.title": "Archived conversations",
          "settings.archived.hint": "Archived conversations stay out of the sidebar. Open, restore, or permanently delete them here.",
          "settings.archived.count": "{count} archived conversations",
          "settings.archived.empty": "No archived conversations.",
          "settings.archived.listAria": "Archived conversations list",
          "settings.archived.deleteWarning": "Delete permanently removes the conversation and cannot be undone.",
          "settings.archived.open": "Open",
          "settings.archived.restore": "Restore",
          "settings.archived.delete": "Delete",
          "settings.archived.unavailableDate": "Archive time unavailable",
          "settings.archived.unknownProject": "No linked project",
          "settings.archived.opened": "Opened archived conversation",
          "settings.archived.restored": "Conversation restored",
          "settings.archived.deleted": "Archived conversation deleted",
          "settings.archived.deleteTitle": "Delete archived conversation",
          "settings.archived.deleteMessage": "Permanently delete conversation “{title}”? This cannot be undone.",
          "settings.archived.busy": "This conversation is already being updated.",
          "settings.providers.hint": "Manage API providers. Third-party sync reads only an export JSON that you explicitly choose; it never scans app or private directories, and keys are never previewed or returned to the UI.",
          "settings.providers.add": "Add provider",
          "settings.providers.refresh": "Refresh",
          "settings.providers.sync": "Import from export JSON",
          "settings.providers.importTitle": "Import preview",
          "settings.providers.importNote": "The selected JSON is parsed only by native code. Keys and custom headers are neither shown nor written by this import. Only public API roots are accepted; configure custom paths manually in the editor. Explicitly choose the configurations to merge.",
          "settings.providers.importSkipped": "Skipped {count} incompatible or duplicate configuration(s).",
          "settings.providers.importCredential": "An export credential was detected; it is neither shown nor written by this import.",
          "settings.providers.importApiRoot": "API root: {path}",
          "settings.providers.importCredentialReentry": "Re-enter local credentials after updating: the imported API root or protocol has changed.",
          "settings.providers.importAdd": "Add",
          "settings.providers.importUpdate": "Update",
          "settings.providers.importSkip": "Skip",
          "settings.providers.importConflict": "A local configuration already exists. It is skipped by default and merges non-sensitive fields only after you select it.",
          "settings.providers.importNew": "A new configuration. Select it before it is added.",
          "settings.providers.importModels": "{count} model(s)",
          "settings.providers.importCancel": "Cancel import",
          "settings.providers.importApply": "Merge selected ({count})",
          "settings.providers.importNone": "Select at least one configuration.",
          "settings.providers.importConfirm": "Merge {count} selected configuration(s)? Only selected entries are added or updated; no key is imported and no network test runs automatically.",
          "settings.providers.importApplied": "Added {added} and updated {updated} provider configuration(s).",
          "settings.providers.importCancelled": "Export JSON selection cancelled.",
          "settings.providers.importFailed": "Could not read export: {error}",
          "settings.providers.listAria": "Provider list",
          "settings.providers.openaiCompat": "OpenAI compatible",
          "settings.providers.test": "Test model",
          "settings.providers.edit": "Edit provider",
          "settings.providers.duplicate": "Duplicate provider",
          "settings.providers.duplicated": "Created a draft copy of “{name}”. Enter an API key, then save.",
          "settings.providers.copySuffix": " (copy)",
          "settings.providers.models": "models",
          "settings.providers.defaultBadge": "Default",
          "settings.providers.statusOk": "Connected",
          "settings.providers.statusConfigured": "Configured",
          "settings.providers.statusTesting": "Testing",
          "settings.providers.statusFailed": "Test failed",
          "settings.providers.statusLocal": "Local",
          "settings.providers.statusNeedKey": "Key needed",
          "settings.providers.statusDraft": "Unsaved",
          "settings.providers.newName": "New provider",
          "settings.providers.newMeta": "https://api.example.com/v1 · Custom protocol",
          "settings.providers.loading": "Reading local provider settings…",
          "settings.providers.empty": "No providers configured. Select Add provider to begin.",
          "settings.providers.unavailable": "Desktop settings are unavailable. Open this view in the NovaVei desktop app.",
          "settings.providers.loadFailed": "Could not read provider settings: {error}",
          "settings.providers.save": "Save configuration",
          "settings.providers.cancel": "Cancel",
          "settings.providers.remove": "Delete provider",
          "settings.providers.name": "Name",
          "settings.providers.id": "ID",
          "settings.providers.type": "Protocol type",
          "settings.providers.baseUrl": "Base URL",
          "settings.providers.modelIds": "Model IDs (one per line)",
          "settings.providers.fetchModels": "Fetch models",
          "settings.providers.fetchModelsLoading": "Fetching models from the saved provider…",
          "settings.providers.fetchModelsSuccess": "Fetched {count} models and placed them in the list. Review, then save.",
          "settings.providers.fetchModelsTruncated": "The upstream result was truncated; only the first {count} models were placed in the list. Review, then save.",
          "settings.providers.fetchModelsEmpty": "The provider returned no usable models. Your current draft is unchanged.",
          "settings.providers.fetchModelsUnsupported": "This provider does not expose a compatible model-list endpoint. Your current draft is unchanged.",
          "settings.providers.fetchModelsSavedOnly": "Connection settings or the API key changed. Save them before fetching models.",
          "settings.providers.fetchModelsUnavailable": "The local model-fetch command is unavailable. Restart NovaVei and try again.",
          "settings.providers.fetchModelsFailed": "Could not fetch models: {error}",
          "settings.providers.apiKey": "API key (leave blank to keep the saved key)",
          "settings.providers.requestFormat": "OpenAI request format",
          "settings.providers.default": "Use as default provider",
          "settings.providers.systemProxy": "Use system proxy",
          "settings.providers.editorTitle": "Edit provider",
          "settings.providers.backToList": "Close",
          "settings.providers.tabGeneral": "General",
          "settings.providers.tabRequest": "Request",
          "settings.providers.basicInfo": "Basic information",
          "settings.providers.enabled": "Enable provider",
          "settings.providers.reentryKey": "Endpoint or auth family changed. Re-enter the API key before saving.",
          "settings.providers.modelCatalogueCheck": "Check connection and model catalogue",
          "settings.providers.modelCatalogueHelp": "This check only probes the model catalogue endpoint. It does not send a billable completion.",
          "settings.providers.modelList": "Models",
          "settings.providers.modelSearch": "Search models…",
          "settings.providers.modelManual": "Add model ID manually",
          "settings.providers.addModel": "Add",
          "settings.providers.requestOptions": "Request options",
          "settings.providers.promptCaching": "Enable prompt caching",
          "settings.providers.promptCachingHelp": "Enable prefix caching where supported. Savings are not guaranteed for every upstream.",
          "settings.providers.customHeaders": "Custom request headers",
          "settings.providers.addHeader": "Add header",
          "settings.providers.headerNote": "Saved header values are never shown. Replace or clear them. Reserved headers such as Authorization and Cookie are rejected.",
          "settings.providers.headerName": "Name",
          "settings.providers.headerValue": "Value",
          "settings.providers.headerConfigured": "Configured",
          "settings.providers.headerClear": "Clear",
          "settings.providers.headerRemove": "Remove",
          "settings.providers.defaultModel": "Default",
          "settings.providers.noModels": "No models yet. Add one manually, or check the connection and fetch the catalogue.",
          "settings.providers.draftPrepareFailed": "Could not prepare connection draft: {error}",
          "settings.providers.catalogueCheckOk": "Model catalogue available · {detail}",
          "settings.providers.catalogueCheckFailed": "Model catalogue check failed: {error}",
          "settings.providers.keyNote": "Keys never return to the UI. Leaving this blank preserves the value held by native.",
          "settings.providers.typeCodex": "OpenAI / Codex",
          "settings.providers.typeClaude": "Anthropic Claude",
          "settings.providers.typeGemini": "Google Gemini",
          "settings.providers.formatResponses": "OpenAI Responses",
          "settings.providers.formatCompletions": "OpenAI Chat Completions",
          "settings.providers.testUnavailable": "The local test command is unavailable. Restart NovaVei and try again.",
          "settings.providers.notConfigured": "“{name}” has no API key configured.",
          "settings.providers.testOk": "“{name}” is available · {detail}",
          "settings.providers.testFailed": "“{name}” test failed: {error}",
          "settings.providers.saved": "Saved provider “{name}”",
          "settings.providers.removed": "Deleted provider “{name}”",
          "settings.providers.invalid": "Enter a valid ID, name, Base URL, and at least one model ID.",
          "settings.system.workdir": "Working directory policy",
          "settings.system.workdirProject": "Limit to project root",
          "settings.system.workdirHelp": "Files, terminal, and workspace capabilities are always bound to the current session's project root. Extra paths are not enabled; open a native-picked folder as a separate project instead.",
          "settings.system.secondaryLaunch": "When NovaVei is launched again",
          "settings.system.secondaryLaunchFocus": "Focus the existing window",
          "settings.system.secondaryLaunchNewWindow": "Open a new window",
          "settings.system.secondaryLaunchHelp": "By default, NovaVei restores the existing window. Opening a new window stays in one background process and shares local data and services.",
          "settings.system.historyPageSize": "History message page size",
          "settings.system.historyPageSizeHelp": "Messages loaded each time a conversation opens; larger values increase first-paint cost.",
          "settings.system.globalSystemPrompt": "Global prompt",
          "settings.system.globalSystemPromptHelp": "Appended to the existing instructions for Chat, Compare, and Council. It consumes model context.",
          "settings.system.globalSystemPromptWarning": "Do not store API keys, passwords, or other credentials here.",
          "settings.system.theme": "Theme",
          "settings.system.themeSystem": "System",
          "settings.system.themeLight": "Light",
          "settings.system.themeDark": "Dark",
          "settings.system.uiScale": "UI scale",
          "settings.system.showShortcutHints": "Show interface shortcut hints",
          "settings.system.showFullMessageTimestamp": "Show full message date",
          "settings.system.userColor": "User message color",
          "settings.system.userColorCustom": "Custom",
          "settings.system.language": "Language",
          "settings.system.languageHint": "Updates settings and shell labels immediately.",
          "settings.mcp.runtime": "MCP runtime",
          "settings.mcp.hint": "MCP configuration and connections are not connected yet.",
          "settings.mcp.openHub": "Open MCP Hub",
          "settings.skills.root": "Skills root",
          "settings.skills.hint": "Skill discovery and installation are not connected yet.",
          "settings.skills.openHub": "Open Skills Hub",
          "settings.cron.title": "Scheduled tasks",
          "settings.cron.hint": "The scheduled-task runtime is not connected; this view is a placeholder.",
          "settings.cron.jobMeta": "Jobs are not loaded",
          "settings.cron.enabled": "Enabled",
          "settings.memory.title": "Memory index",
          "settings.memory.hint": "Markdown facts + SQLite FTS. Scopes: global / project / daily.",
          "settings.memory.entries": "Entries",
          "settings.memory.today": "Today",
          "settings.memory.quota": "Quota",
          "settings.council.title": "Expert templates",
          "settings.council.hint": "The Council runtime is not connected; this view is a placeholder.",
          "settings.council.arch": "Architecture chair",
          "settings.council.archMeta": "Synthesis · read-only",
          "settings.council.security": "Security advisor",
          "settings.council.securityMeta": "Threat modeling · read-only",
          "settings.council.growth": "Growth advisor",
          "settings.council.growthMeta": "Product tradeoffs · read-only",
          "settings.council.builtin": "Built-in",
          "settings.council.optional": "Optional",
          "settings.about.blurb": "Tauri 2 + TypeScript + Rust + embedded Pi",
          "settings.about.skin": "Skin: Luminous Quiet · design plan4",
          "settings.toast.langZh": "已切换为简体中文",
          "settings.toast.langEn": "Switched to English",
          "theme.toLight": "Switch to light theme",
          "theme.toDark": "Switch to dark theme",
          "theme.switched": "Switched to",
          "settings.system.tabsAria": "System settings groups",
          "settings.system.tabAppearance": "Appearance",
          "settings.system.tabBehavior": "Behavior",
          "settings.system.tabPortable": "Portable",
          "settings.portable.title": "Run mode",
          "settings.portable.description": "Choose whether the next launch uses the installed app-data directory or the portable directory beside the EXE.",
          "settings.portable.optionsAria": "Choose run mode",
          "settings.portable.installedTitle": "Installed",
          "settings.portable.installedDescription": "Data stays in this Windows user's application-data directory.",
          "settings.portable.portableTitle": "Portable",
          "settings.portable.portableDescription": "Data stays in the novavei folder beside the EXE and is password-protected at startup.",
          "settings.portable.isolation": "Switching never moves, copies, or deletes existing chats, settings, or credentials; the two modes keep separate data.",
          "settings.portable.apply": "Switch run mode",
          "settings.system.uiFont": "UI font",
          "settings.system.codeFont": "Code font",
          "settings.system.fontSystem": "System default",
          "settings.nav.shortcuts": "Shortcuts",
          "settings.shortcuts.title": "Keyboard shortcuts",
          "settings.shortcuts.hint": "The home screen shows common shortcut hints. Turn them off here; the shortcuts still work.",
          "settings.shortcuts.search": "Search chats & projects",
          "settings.shortcuts.findConversation": "Find in current conversation",
          "settings.shortcuts.newChat": "New chat",
          "settings.shortcuts.settings": "Open settings",
          "settings.shortcuts.theme": "Toggle light / dark",
          "settings.shortcuts.dock": "Show / hide right dock",
          "settings.shortcuts.conversationOnly": "Show conversation only",
          "settings.shortcuts.escape": "Close overlays / search / popovers",
          "shortcut.conversationOnly.entered": "Conversation-only mode enabled",
          "shortcut.conversationOnly.exited": "Conversation-only mode disabled",
          "settings.memory.tabsAria": "Memory types",
          "settings.memory.tabProject": "Project",
          "settings.memory.tabLongterm": "Long-term",
          "settings.memory.tabKnowledge": "Knowledge base",
          "settings.memory.tabUsage": "Usage",
          "settings.memory.knowledgeTitle": "Knowledge base",
          "settings.memory.knowledgeHint": "Knowledge bases load when the native runtime is available.",
          "settings.memory.projectTitle": "Project memory",
          "settings.memory.projectHint": "The project-memory runtime is not connected yet.",
          "settings.memory.projectRoot": "Current project",
          "settings.memory.projectRootHint": "No project-memory path has been loaded from the runtime.",
          "settings.memory.longTitle": "Long-term memory",
          "settings.memory.longHint": "The long-term-memory runtime is not connected yet.",
          "settings.memory.longPath": "Storage path",
          "settings.memory.organize": "Organize",
          "settings.memory.export": "Export",
          "settings.memory.clear": "Clear",
          "settings.memory.usageTitle": "Usage",
          "settings.memory.usageHint": "Memory statistics are not connected; no usage data is available.",
          "settings.memory.usageTotal": "Total entries",
          "settings.memory.usageProject": "Project",
          "settings.memory.usageLong": "Long-term",
          "settings.memory.usageStorage": "Disk usage",
          "settings.memory.usageHits": "Retrievals (week)",
          "settings.memory.usageWrites": "Writes (week)",
          "settings.memory.usageQuotaLabel": "Quota",
          "settings.memory.usageQuotaHint": "No quota data available",
          "settings.memory.usageRefresh": "Refresh",
          "settings.memory.usageReport": "Export report",
          "${statusKey}": "${statusKey}",
          "settings.cron.paused": "Paused",
          "settings.cron.refresh": "Refresh",
          "settings.cron.create": "New job",
          "settings.cron.statTotal": "All jobs",
          "settings.cron.statEnabled": "Enabled",
          "settings.cron.statPaused": "Paused",
          "settings.cron.runNow": "Run now",
          "settings.cron.edit": "Edit",
          "settings.cron.logs": "Logs",
          "settings.cron.resume": "Enable",
          "settings.cron.job1Desc": "Jobs are not loaded from the runtime",
          "settings.cron.job2Desc": "Jobs are not loaded from the runtime",
          "settings.cron.job3Desc": "Jobs are not loaded from the runtime",
          "settings.cron.lastOk": "No run history",
          "settings.cron.lastOkWeek": "No run history",
          "settings.cron.lastSkip": "No run history",
          "settings.council.tabsAria": "Experts settings",
          "settings.council.tabExperts": "Experts",
          "settings.council.tabCharacters": "Characters",
          "settings.council.tabTeams": "Teams",
          "settings.council.tabPrompts": "Custom prompts",
          "settings.council.expertsTitle": "Experts",
          "settings.council.expertsHint": "Expert configuration has not been loaded from the Council runtime.",
          "settings.council.addExpert": "Add expert",
          "settings.council.charactersTitle": "Characters",
          "settings.council.charactersHint": "Character configuration has not been loaded from the Council runtime.",
          "settings.council.addCharacter": "Create character",
          "settings.council.charactersLoading": "Character configuration has not been loaded",
          "settings.council.charactersLoadingHint": "Waiting for local character storage",
          "settings.council.unavailable": "Unavailable",
          "settings.council.eng": "Engineering advisor",
          "settings.council.engMeta": "Implementation path · write suggestions",
          "settings.council.teamsTitle": "Expert teams",
          "settings.council.teamsHint": "Team configuration has not been loaded from the Council runtime.",
          "settings.council.addTeam": "Create team",
          "settings.council.team1": "Architecture review",
          "settings.council.team1Meta": "Architecture · Security · Engineering · Product",
          "settings.council.team2": "Release gate",
          "settings.council.team2Meta": "Security · Engineering · Growth",
          "settings.council.team3": "Product discovery",
          "settings.council.team3Meta": "Growth · Product · Architecture",
          "settings.council.promptsTitle": "Custom prompts",
          "settings.council.promptsHint": "Prompt storage has not been loaded from the Council runtime.",
          "settings.council.addPrompt": "New prompt",
          "settings.council.prompt1": "Architecture · strict",
          "settings.council.prompt2": "Security checklist",
          "settings.council.override": "Override",
          "settings.council.custom": "Custom",
          "settings.council.edit": "Edit",
          "settings.council.copy": "Copy"
        },
      };

      let lang = "zh";
      function t(key) {
        const fromLang = I18N[lang] && I18N[lang][key];
        const fromZh = I18N.zh && I18N.zh[key];
        const v = (fromLang != null && fromLang !== "" ? fromLang : null)
          || (fromZh != null && fromZh !== "" ? fromZh : null);
        if (v != null && !/^[a-z0-9]+(\.[a-z0-9]+)+$/i.test(String(v))) return v;
        return v != null ? v : key;
      }

      const I18N_SELECTOR = "[data-i18n], [data-i18n-aria], [data-i18n-title], [data-toast-key]";

      function translateI18nText(el) {
        const key = el.getAttribute("data-i18n");
        if (!key) return;
        const val = t(key);
        // Unknown keys can be used by runtime-owned nodes. Keep their real
        // content instead of replacing it with a dictionary key.
        if (val === key && el.textContent && el.textContent.trim() && el.textContent.trim() !== key) return;
        el.textContent = val;
      }

      function translateI18nAria(el) {
        const key = el.getAttribute("data-i18n-aria");
        if (key) el.setAttribute("aria-label", t(key));
      }

      function translateI18nTitle(el) {
        const key = el.getAttribute("data-i18n-title");
        if (key) el.setAttribute("title", t(key));
      }

      function translateI18nToast(el) {
        const key = el.getAttribute("data-toast-key");
        if (key) el.dataset.toast = t(key);
      }

      const i18nBindings = [
        { attribute: "data-i18n", nodes: new Set(), translate: translateI18nText },
        { attribute: "data-i18n-aria", nodes: new Set(), translate: translateI18nAria },
        { attribute: "data-i18n-title", nodes: new Set(), translate: translateI18nTitle },
        { attribute: "data-toast-key", nodes: new Set(), translate: translateI18nToast },
      ];

      function bindingForI18nAttribute(attribute) {
        return i18nBindings.find((binding) => binding.attribute === attribute);
      }

      function syncI18nBinding(el, binding, translate) {
        if (el.hasAttribute(binding.attribute)) {
          binding.nodes.add(el);
          if (translate) binding.translate(el);
        } else {
          binding.nodes.delete(el);
        }
      }

      function registerI18nElement(el, translate) {
        i18nBindings.forEach((binding) => syncI18nBinding(el, binding, translate));
      }

      function registerI18nTree(root, translate) {
        if (!(root instanceof Element)) return;
        registerI18nElement(root, translate);
        root.querySelectorAll(I18N_SELECTOR).forEach((el) => {
          registerI18nElement(el, translate);
        });
      }

      function unregisterI18nElement(el) {
        i18nBindings.forEach((binding) => binding.nodes.delete(el));
      }

      function unregisterI18nTree(root) {
        if (!(root instanceof Element)) return;
        unregisterI18nElement(root);
        root.querySelectorAll(I18N_SELECTOR).forEach(unregisterI18nElement);
      }

      function startI18nRegistry() {
        // One startup scan registers the static shell. Language changes then
        // iterate only translatable nodes rather than the entire document.
        registerI18nTree(document.documentElement, false);
        if (typeof MutationObserver === "undefined") return;
        const observer = new MutationObserver((records) => {
          records.forEach((record) => {
            if (record.type === "childList") {
              record.removedNodes.forEach(unregisterI18nTree);
              record.addedNodes.forEach((node) => registerI18nTree(node, true));
              return;
            }
            const binding = bindingForI18nAttribute(record.attributeName);
            if (binding && record.target instanceof Element) {
              syncI18nBinding(record.target, binding, true);
            }
          });
        });
        observer.observe(document.documentElement, {
          childList: true,
          subtree: true,
          attributes: true,
          attributeFilter: i18nBindings.map((binding) => binding.attribute),
        });
      }

      function applyI18n() {
        document.documentElement.lang = lang === "en" ? "en" : "zh-CN";
        i18nBindings.forEach((binding) => {
          binding.nodes.forEach((el) => {
            if (!el.isConnected) {
              binding.nodes.delete(el);
              return;
            }
            binding.translate(el);
          });
        });
        const settingsRoot = document.getElementById("overlaySettings");
        if (settingsRoot) settingsRoot.setAttribute("aria-label", t("settings.title"));
        syncLangSeg();
        syncThemeSeg();
        syncThemeButton();
      }

      function setLanguage(next, announce) {
        lang = next === "en" ? "en" : "zh";
        applyI18n();
        if (typeof renderProviders === "function") renderProviders();
        syncLangSeg();
        // Runtime modules render dynamic labels outside the static
        // data-i18n dictionary. Publish a single shell event so File Dock,
        // permissions, and similar surfaces can refresh without polling.
        window.dispatchEvent(new CustomEvent("novavei:language-changed", { detail: { lang } }));
        if (announce) toast(t(lang === "en" ? "settings.toast.langEn" : "settings.toast.langZh"));
      }

      function syncThemeButton() {
        const theme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
        const btn = document.getElementById("btnTheme");
        if (!btn) return;
        btn.textContent = theme === "dark" ? "Dark" : "Light";
        const label = theme === "dark" ? t("theme.toLight") : t("theme.toDark");
        btn.setAttribute("aria-label", label);
        btn.title = label;
      }

      let themePref = "system";
      function syncThemeSeg() {
        document.querySelectorAll("[data-theme-opt]").forEach((btn) => {
          btn.classList.toggle("on", btn.dataset.themeOpt === themePref);
        });
      }
      function syncLangSeg() {
        document.querySelectorAll("[data-lang-opt]").forEach((btn) => {
          btn.classList.toggle("on", btn.dataset.langOpt === lang);
        });
      }
      function setTheme(theme, opts = {}) {
        document.documentElement.dataset.theme = theme;
        if (opts.pref) themePref = opts.pref;
        else if (themePref !== "system") themePref = theme;
        syncThemeButton();
        syncThemeSeg();
        if (opts.silent) return;
        toast(t("theme.switched") + " " + (theme === "light" ? "Light" : "Dark") + " · Luminous Quiet");
      }
      let uiScale = 100;
      let userAccent = "#0A84FF";
      function syncUiScaleSeg() {
        document.querySelectorAll("[data-ui-scale]").forEach((btn) => {
          btn.classList.toggle("on", Number(btn.dataset.uiScale) === uiScale);
        });
      }
      function setUiScale(pct, announce) {
        uiScale = Number(pct) || 100;
        document.documentElement.style.setProperty("--ui-scale", String(uiScale / 100));
        document.documentElement.style.zoom = uiScale + "%";
        syncUiScaleSeg();
        if (announce) toast((lang === "en" ? "UI scale: " : "界面大小：") + uiScale + "%");
      }
      function hexToRgbTuple(hex) {
        const raw = String(hex || "").replace("#", "");
        const full = raw.length === 3 ? raw.split("").map((c) => c + c).join("") : raw;
        const n = parseInt(full, 16);
        if (Number.isNaN(n)) return [10, 132, 255];
        return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
      }
      function applyUserAccent(hex, announce) {
        userAccent = hex;
        const [r, g, b] = hexToRgbTuple(hex);
        document.documentElement.style.setProperty("--user-accent", hex);
        const darkBubble = "linear-gradient(160deg, rgb(" + r + " " + g + " " + b + " / 28%), rgb(40 52 74 / 75%))";
        const lightBubble = "linear-gradient(160deg, rgb(" + r + " " + g + " " + b + " / 15%), rgb(232 243 255 / 94%))";
        const isLight = document.documentElement.dataset.theme === "light";
        document.documentElement.style.setProperty("--user-bubble", isLight ? lightBubble : darkBubble);
        document.querySelectorAll("[data-user-color]").forEach((btn) => {
          const same = (btn.dataset.userColor || "").toLowerCase() === hex.toLowerCase();
          btn.classList.toggle("on", same);
          btn.setAttribute("aria-pressed", String(same));
        });
        const picker = document.getElementById("userColorPicker");
        if (picker) picker.value = hex.length === 4 || hex.length === 7 ? hex : "#0A84FF";
        if (announce) toast((lang === "en" ? "User message color updated" : "用户消息颜色已更新"));
      }
      // re-apply bubble when theme flips
      const _setThemeOrig = setTheme;
      setTheme = function(theme, opts = {}) {
        _setThemeOrig(theme, opts);
        applyUserAccent(userAccent, false);
      };
      const systemThemeQuery = typeof window.matchMedia === "function"
        ? window.matchMedia("(prefers-color-scheme: light)")
        : null;
      function applySystemThemeChange(event) {
        if (themePref !== "system") return;
        setTheme(event.matches ? "light" : "dark", { pref: "system", silent: true });
      }
      if (systemThemeQuery) {
        if (typeof systemThemeQuery.addEventListener === "function") {
          systemThemeQuery.addEventListener("change", applySystemThemeChange);
        } else if (typeof systemThemeQuery.addListener === "function") {
          systemThemeQuery.addListener(applySystemThemeChange);
        }
      }

      let toastTimer = 0;
      function toast(msg) {
        toastEl.textContent = msg;
        toastEl.classList.add("show");
        clearTimeout(toastTimer);
        toastTimer = setTimeout(() => toastEl.classList.remove("show"), 1800);
      }

      startI18nRegistry();
      applyI18n();

      function closeOverlays() {
        if (typeof closeProviderEditorView === "function" && document.getElementById("providerEditorModal")?.classList.contains("show")) {
          closeProviderEditorView();
        }
        const hadOpenOverlay = Object.values(overlays).some((el) => el.classList.contains("show"));
        Object.values(overlays).forEach((el) => el.classList.remove("show"));
        if (hadOpenOverlay) document.getElementById("composerInput").focus();
      }

      function openOverlay(name) {
        Object.entries(overlays).forEach(([key, el]) => {
          el.classList.toggle("show", key === name);
        });
        const activeOverlay = overlays[name];
        requestAnimationFrame(() => activeOverlay?.querySelector("button, input, select")?.focus());
      }

      function setProjectExpanded(project, expanded) {
        const conversations = document.getElementById(project.getAttribute("aria-controls"));
        project.setAttribute("aria-expanded", String(expanded));
        if (conversations) conversations.hidden = !expanded;
      }

      function setCurrentProject(project) {
        document.querySelectorAll(".project-row").forEach((item) => {
          item.removeAttribute("aria-current");
          const state = item.querySelector(".project-state");
          if (state) state.textContent = item.dataset.state || "项目";
        });
        project.setAttribute("aria-current", "page");
        project.querySelector(".project-state").textContent = "当前";
        const name = project.dataset.project || project.querySelector("strong")?.textContent || "项目";
        const workdir = project.dataset.workdir || name;
        const workdirStatus = document.getElementById("workdirStatus");
        if (workdirStatus) workdirStatus.textContent = workdir;
        // The preview shell only announces its visual selection. In Tauri,
        // permission-picker.ts reads the native host workdir as authority.
        window.dispatchEvent(
          new CustomEvent("novavei:current-project-changed", {
            detail: { workdir },
          }),
        );
        return name;
      }

      document.querySelectorAll(".project-row").forEach((project) => {
        setProjectExpanded(project, project.dataset.expanded === "true");
        project.addEventListener("click", () => {
          const nextExpanded = project.getAttribute("aria-expanded") !== "true";
          setProjectExpanded(project, nextExpanded);
          project.dataset.expanded = String(nextExpanded);
          const name = setCurrentProject(project);
          chatTitle.textContent = name + " · 新建对话";
          toast((nextExpanded ? "已展开" : "已收起") + "项目文件夹：" + name);
        });
      });

      // Session select
      function activateSession(btn, announce) {
        if (!btn) return;
        document.querySelectorAll(".session").forEach((s) => s.classList.remove("active"));
        btn.classList.add("active");
        const project = btn.closest(".project-folder")?.querySelector(".project-row")
          || document.querySelector(`.project-row[data-project="${btn.dataset.project}"]`);
        if (project) setCurrentProject(project);
        chatTitle.textContent = btn.dataset.title || btn.querySelector("strong")?.textContent || "";
        closeOverlays();
        if (window.matchMedia("(max-width: 820px)").matches) workbench.classList.remove("side-open");
        syncSideToggle();
        if (announce) toast(lang === "en" ? "Switched conversation" : "已切换会话");
      }
      document.querySelectorAll(".session").forEach((btn) => {
        btn.addEventListener("click", () => activateSession(btn, true));
      });

      // Sidebar context menus: session copy/delete + project remove/copy path
      (function initSidebarContextMenus() {
        const sessionMenu = document.getElementById("sessionCtxMenu");
        const projectMenu = document.getElementById("projectCtxMenu");
        if (!sessionMenu && !projectMenu) return;

        let targetSession = null;
        let targetProject = null;
        const SESSION_TITLE_MAX_LENGTH = 200;

        function clearCtxHighlight() {
          document
            .querySelectorAll(".session.is-ctx-target, .project-row.is-ctx-target")
            .forEach((el) => el.classList.remove("is-ctx-target"));
        }

        function closeAllMenus() {
          if (sessionMenu) sessionMenu.hidden = true;
          if (projectMenu) projectMenu.hidden = true;
          targetSession = null;
          targetProject = null;
          clearCtxHighlight();
        }

        function placeMenu(menu, clientX, clientY) {
          menu.hidden = false;
          const pad = 8;
          const rect = menu.getBoundingClientRect();
          let left = clientX;
          let top = clientY;
          if (left + rect.width > window.innerWidth - pad) {
            left = Math.max(pad, window.innerWidth - rect.width - pad);
          }
          if (top + rect.height > window.innerHeight - pad) {
            top = Math.max(pad, window.innerHeight - rect.height - pad);
          }
          menu.style.left = left + "px";
          menu.style.top = top + "px";
        }

        function openOn(menu, el, clientX, clientY, ariaKey) {
          // Hide other menus without wiping the target refs that the caller just set.
          if (sessionMenu) sessionMenu.hidden = true;
          if (projectMenu) projectMenu.hidden = true;
          clearCtxHighlight();
          if (!menu || !el) return;
          el.classList.add("is-ctx-target");
          menu.setAttribute("aria-label", t(ariaKey));
          placeMenu(menu, clientX, clientY);
          requestAnimationFrame(() => placeMenu(menu, clientX, clientY));
        }

        function sessionTitle(btn) {
          return (
            btn?.dataset?.title ||
            btn?.querySelector("strong")?.textContent?.trim() ||
            ""
          );
        }

        function sessionMeta(btn) {
          return (
            btn?.dataset?.meta ||
            btn?.querySelector("small")?.textContent?.replace(/\s+/g, " ").trim() ||
            ""
          );
        }

        function isPinnedSession(btn) {
          return Boolean(
            btn?.closest('[data-session-group="pinned"]') ||
              btn?.classList.contains("is-pinned-entry"),
          );
        }

        function isArchivedSession(btn) {
          return Boolean(btn?.classList.contains("is-archived-entry"));
        }

        function ensurePinnedGroup() {
          let group = document.getElementById("pinnedSessionsGroup");
          if (group) {
            group.hidden = false;
            return group;
          }
          const list = document.getElementById("sessionList");
          if (!list) return null;
          group = document.createElement("section");
          group.className = "session-group";
          group.dataset.sessionGroup = "pinned";
          group.id = "pinnedSessionsGroup";
          group.setAttribute("aria-labelledby", "pinnedSessionsLabel");
          group.innerHTML =
            '<h3 class="group-label" id="pinnedSessionsLabel">' +
            t("session.groupPinned") +
            "</h3>";
          list.prepend(group);
          return group;
        }

        function ensureArchivedGroup() {
          return null;
        }

        function refreshGroupVisibility() {
          const pinned = document.getElementById("pinnedSessionsGroup");
          if (pinned) pinned.hidden = !pinned.querySelector(".session");
        }

        function resolveProjectName(btn) {
          if (btn?.dataset?.project) return btn.dataset.project;
          const row = btn?.closest(".project-folder")?.querySelector(".project-row");
          return row?.dataset?.project || "";
        }

        function bindSessionClick(btn) {
          if (!btn || btn.dataset.boundClick === "1") return;
          btn.dataset.boundClick = "1";
          btn.addEventListener("click", () => activateSession(btn, true));
        }

        function cloneSessionNode(btn) {
          const clone = btn.cloneNode(true);
          clone.classList.remove("active", "is-ctx-target");
          clone.removeAttribute("data-bound-click");
          bindSessionClick(clone);
          return clone;
        }

        function findSessionsByTitle(title) {
          return [...document.querySelectorAll(".session")].filter(
            (s) => sessionTitle(s) === title,
          );
        }

        function syncSessionMenuLabels(btn) {
          const pinBtn = document.getElementById("sessionCtxPin");
          const archiveBtn = document.getElementById("sessionCtxArchive");
          const pinned = isPinnedSession(btn);
          const archived = isArchivedSession(btn);
          if (pinBtn) {
            pinBtn.dataset.sessionAction = pinned ? "unpin" : "pin";
            pinBtn.hidden = archived;
            const pinLabel = pinBtn.querySelector("span");
            if (pinLabel) pinLabel.textContent = t(pinned ? "session.unpin" : "session.pin");
          }
          if (archiveBtn) {
            archiveBtn.dataset.sessionAction = "archive";
            const archLabel = archiveBtn.querySelector("span");
            if (archLabel) archLabel.textContent = t("session.archive");
          }
        }

        function pinSession(btn) {
          if (!btn || isArchivedSession(btn)) return;
          const title = sessionTitle(btn);
          const group = ensurePinnedGroup();
          if (!group) return;
          const already = [...group.querySelectorAll(".session")].some(
            (s) => sessionTitle(s) === title,
          );
          if (already) {
            toast(t("session.pinned"));
            return;
          }
          const project = resolveProjectName(btn);
          const entry = cloneSessionNode(btn);
          entry.classList.add("is-pinned-entry");
          entry.classList.remove("is-archived-entry");
          if (project) entry.dataset.project = project;
          if (!entry.dataset.title) entry.dataset.title = title;
          group.appendChild(entry);
          refreshGroupVisibility();
          toast(t("session.pinned"));
        }

        function unpinSession(btn) {
          const title = sessionTitle(btn);
          const group = document.getElementById("pinnedSessionsGroup");
          if (!group) return;
          [...group.querySelectorAll(".session")]
            .filter((s) => sessionTitle(s) === title)
            .forEach((s) => s.remove());
          // Also clear pin styling from other instances
          findSessionsByTitle(title).forEach((s) => {
            if (!s.closest('[data-session-group="pinned"]')) {
              s.classList.remove("is-pinned-entry");
            }
          });
          refreshGroupVisibility();
          toast(t("session.unpinned"));
        }

        function archiveSession(btn) {
          if (!btn) return;
          const title = sessionTitle(btn);
          const wasActive =
            btn.classList.contains("active") ||
            chatTitle.textContent.trim() === title;

          // Remove from pin + project lists completely (no sidebar archived group)
          findSessionsByTitle(title).forEach((s) => s.remove());
          refreshGroupVisibility();

          if (wasActive) {
            const next =
              document.querySelector(".project-conversations .session") ||
              document.querySelector('[data-session-group="pinned"] .session') ||
              document.querySelector(".session");
            if (next) activateSession(next, false);
          }
          toast(t("session.archived"));
          openOverlay("settings");
          if (typeof goToSettings === "function") {
            goToSettings("archived", { animate: false });
          } else {
            const navBtn = document.querySelector('.settings-nav button[data-settings="archived"]');
            navBtn?.click();
          }
        }

        function unarchiveSession() {
          // Archived conversations are managed in Settings; browser preview no longer unarchives from the sidebar.
        }

        function projectName(row) {
          return (
            row?.dataset?.project ||
            row?.querySelector("strong")?.textContent?.trim() ||
            ""
          );
        }

        function projectPath(row) {
          return (
            row?.dataset?.workdir ||
            row?.getAttribute("title") ||
            row?.querySelector("small")?.textContent?.trim() ||
            projectName(row)
          );
        }

        async function writeClipboard(text) {
          if (navigator.clipboard?.writeText) {
            await navigator.clipboard.writeText(text);
            return;
          }
          const ta = document.createElement("textarea");
          ta.value = text;
          ta.style.position = "fixed";
          ta.style.left = "-9999px";
          document.body.appendChild(ta);
          ta.select();
          document.execCommand("copy");
          ta.remove();
        }

        function buildConversationExport(btn) {
          const title = sessionTitle(btn);
          const meta = sessionMeta(btn);
          const lines = [title];
          if (meta) lines.push(meta);
          const isActive =
            btn.classList.contains("active") ||
            chatTitle.textContent.trim() === title;
          if (isActive) {
            const axis = document.getElementById("transcriptAxis")
              || document.querySelector(".axis");
            if (axis) {
              lines.push("");
              axis.querySelectorAll(".msg-user, .msg-assistant").forEach((node) => {
                if (node.classList.contains("msg-user")) {
                  lines.push("User: " + (node.textContent || "").replace(/\s+/g, " ").trim());
                } else {
                  const who = node.querySelector(".who b")?.textContent?.trim() || "Assistant";
                  const body = [...node.querySelectorAll("p, .principle, pre")]
                    .map((el) => el.textContent.replace(/\s+/g, " ").trim())
                    .filter(Boolean)
                    .join("\n");
                  lines.push(who + ": " + (body || node.textContent.replace(/\s+/g, " ").trim()));
                }
                lines.push("");
              });
            }
          }
          return lines.join("\n").trim();
        }

        async function copySession(btn) {
          try {
            await writeClipboard(buildConversationExport(btn));
            toast(t("session.copied"));
          } catch {
            toast(lang === "en" ? "Copy failed" : "复制失败");
          }
        }


        function appDialogs() {
          return window.__novaveiDialogs;
        }
        async function appConfirm(options) {
          const dialogs = appDialogs();
          if (!dialogs?.confirm) {
            console.warn("[NovaVei] in-app confirm unavailable");
            return false;
          }
          return dialogs.confirm(options);
        }
        async function appPrompt(options) {
          const dialogs = appDialogs();
          if (!dialogs?.prompt) {
            console.warn("[NovaVei] in-app prompt unavailable");
            return null;
          }
          return dialogs.prompt(options);
        }
        async function appError(error, title) {
          const dialogs = appDialogs();
          if (!dialogs?.error) {
            console.error("[NovaVei]", title || "error", error);
            toast(String(error?.message || error || "操作失败"));
            return;
          }
          await dialogs.error(error, title);
        }

        async function renameSession(btn) {
          if (!btn) return;
          const previousTitle = sessionTitle(btn);
          const entered = await appPrompt({
            title: t("session.renameDialogTitle"),
            label: t("session.renameLabel"),
            initialValue: previousTitle,
            maxLength: SESSION_TITLE_MAX_LENGTH,
          });
          if (entered === null) return;
          const title = entered.trim();
          if (!title) {
            toast(t("session.renameEmpty"));
            return;
          }
          if (Array.from(title).length > SESSION_TITLE_MAX_LENGTH) {
            toast(t("session.renameTooLong"));
            return;
          }
          if (title === previousTitle) return;
          findSessionsByTitle(previousTitle).forEach((session) => {
            session.dataset.title = title;
            const label = session.querySelector("strong");
            if (label) label.textContent = title;
          });
          if (chatTitle.textContent.trim() === previousTitle) chatTitle.textContent = title;
          toast(t("session.renamed"));
        }

        function deleteSession(btn) {
          if (!btn) return;
          const title = sessionTitle(btn);
          const wasActive =
            btn.classList.contains("active") ||
            chatTitle.textContent.trim() === title;
          const all = [...document.querySelectorAll(".session")].filter(
            (s) => sessionTitle(s) === title,
          );
          (all.length ? all : [btn]).forEach((el) => el.remove());
          document.querySelectorAll(".session-group").forEach((group) => {
            if (!group.querySelector(".session")) group.hidden = true;
          });
          if (wasActive) {
            const next =
              document.querySelector(".session.active") ||
              document.querySelector(".project-conversations .session") ||
              document.querySelector(".session");
            if (next) {
              activateSession(next, false);
              toast(t("session.deleteActive"));
            } else {
              chatTitle.textContent = lang === "en" ? "New chat" : "新建对话";
              toast(t("session.deleted"));
            }
          } else {
            toast(t("session.deleted"));
          }
        }

        async function copyProjectPath(row) {
          try {
            await writeClipboard(projectPath(row));
            toast(t("project.pathCopied"));
          } catch {
            toast(lang === "en" ? "Copy failed" : "复制失败");
          }
        }

        async function removeProjectFolder(row) {
          if (!row) return;
          const folder = row.closest(".project-folder");
          if (!folder) return;
          const name = projectName(row);
          const wasCurrent = row.getAttribute("aria-current") === "page";
          const workdir = row.dataset.workdir?.trim();
          if (workdir && window.__novaveiHost?.removeProject) {
            try {
              const result = await window.__novaveiHost.removeProject(workdir);
              if (!result?.removed) {
                toast(lang === "en" ? "The project is not in the saved project list." : "该项目不在已保存的项目列表中。");
                return;
              }
              toast(result.wasCurrent ? t("project.removedCurrent") : t("project.removed"));
            } catch (error) {
              toast(lang === "en" ? "Could not remove the saved project." : "移除项目失败，项目列表未改变。");
            }
            return;
          }
          const titlesInFolder = [
            ...folder.querySelectorAll(".project-conversations .session"),
          ].map(sessionTitle).filter(Boolean);

          // Drop matching pinned sessions that belong to this project
          document.querySelectorAll(".session-list .session, .session-group .session").forEach((s) => {
            const sameTitle = titlesInFolder.includes(sessionTitle(s));
            const sameProject = s.dataset.project && s.dataset.project === name;
            if (sameTitle || sameProject) s.remove();
          });
          document.querySelectorAll(".session-group").forEach((group) => {
            if (!group.querySelector(".session")) group.hidden = true;
          });

          folder.remove();

          const projectList = document.querySelector(".project-list");
          if (projectList && !projectList.querySelector(".project-folder")) {
            const empty = document.getElementById("sidebarEmpty");
            if (empty) {
              empty.hidden = false;
              empty.textContent =
                lang === "en"
                  ? "No project folders. Open a folder to get started."
                  : "还没有项目文件夹。点击「打开」添加一个。";
            }
          }

          if (wasCurrent) {
            const nextRow = document.querySelector(".project-row");
            if (nextRow) {
              setCurrentProject(nextRow);
              setProjectExpanded(nextRow, true);
              nextRow.dataset.expanded = "true";
              const nextSession =
                nextRow.closest(".project-folder")?.querySelector(".session") ||
                document.querySelector(".session");
              if (nextSession) activateSession(nextSession, false);
              else chatTitle.textContent = projectName(nextRow);
              toast(t("project.removedCurrent"));
            } else {
              chatTitle.textContent = lang === "en" ? "New chat" : "新建对话";
              const workdirStatus = document.getElementById("workdirStatus");
              if (workdirStatus) {
                workdirStatus.textContent =
                  lang === "en" ? "No project open" : "未打开项目";
              }
              toast(t("project.removedCurrent"));
            }
          } else {
            toast(t("project.removed"));
          }
        }

        const sidebar = document.getElementById("sessionSidebar");

        document.addEventListener("contextmenu", (e) => {
          const el = e.target instanceof Element ? e.target : null;
          if (!el || !sidebar?.contains(el)) {
            closeAllMenus();
            return;
          }
          // Prefer session over project when nested (conversations inside folder)
          const sessionBtn = el.closest(".session");
          if (sessionBtn && sidebar.contains(sessionBtn)) {
            e.preventDefault();
            e.stopPropagation();
            targetSession = sessionBtn;
            targetProject = null;
            syncSessionMenuLabels(sessionBtn);
            openOn(sessionMenu, sessionBtn, e.clientX, e.clientY, "session.ctxAria");
            return;
          }
          const projectRow = el.closest(".project-row");
          if (projectRow && sidebar.contains(projectRow)) {
            e.preventDefault();
            e.stopPropagation();
            targetProject = projectRow;
            targetSession = null;
            openOn(projectMenu, projectRow, e.clientX, e.clientY, "project.ctxAria");
            return;
          }
          closeAllMenus();
        });

        sessionMenu?.addEventListener("click", (e) => {
          const actionBtn = e.target instanceof Element
            ? e.target.closest("[data-session-action]")
            : null;
          if (!actionBtn || !targetSession) return;
          const action = actionBtn.dataset.sessionAction;
          const session = targetSession;
          closeAllMenus();
          if (action === "copy") copySession(session);
          else if (action === "rename") renameSession(session);
          else if (action === "delete") deleteSession(session);
          else if (action === "pin") pinSession(session);
          else if (action === "unpin") unpinSession(session);
          else if (action === "archive") archiveSession(session);
          else if (action === "unarchive") { /* no-op: managed in Settings */ }
        });

        refreshGroupVisibility();

        projectMenu?.addEventListener("click", (e) => {
          const actionBtn = e.target instanceof Element
            ? e.target.closest("[data-project-action]")
            : null;
          if (!actionBtn || !targetProject) return;
          const action = actionBtn.dataset.projectAction;
          const project = targetProject;
          closeAllMenus();
          if (action === "copy-path") copyProjectPath(project);
          else if (action === "remove") removeProjectFolder(project);
        });

        document.addEventListener(
          "pointerdown",
          (e) => {
            const t = e.target;
            if (!(t instanceof Node)) return;
            if (sessionMenu && !sessionMenu.hidden && sessionMenu.contains(t)) return;
            if (projectMenu && !projectMenu.hidden && projectMenu.contains(t)) return;
            if (
              (sessionMenu && !sessionMenu.hidden) ||
              (projectMenu && !projectMenu.hidden)
            ) {
              closeAllMenus();
            }
          },
          true,
        );
        document.addEventListener("keydown", (e) => {
          if (e.key !== "Escape") return;
          if (
            (sessionMenu && !sessionMenu.hidden) ||
            (projectMenu && !projectMenu.hidden)
          ) {
            e.preventDefault();
            closeAllMenus();
          }
        });
        window.addEventListener("blur", closeAllMenus);
        window.addEventListener("resize", closeAllMenus);
        sidebar?.addEventListener("scroll", closeAllMenus, true);
      })();

      function syncSideToggle() {
        const isMobile = window.matchMedia("(max-width: 820px)").matches;
        const conversationOnly = workbench.classList.contains("conversation-only");
        const expanded = !conversationOnly && (isMobile ? workbench.classList.contains("side-open") : !workbench.classList.contains("side-closed"));
        document.getElementById("btnToggleSide").setAttribute("aria-expanded", String(expanded));
      }
      function toggleConversationOnlyMode() {
        const conversationOnly = workbench.classList.toggle("conversation-only");
        if (conversationOnly) {
          // A focusable element must not remain in a panel that was just hidden.
          const active = document.activeElement;
          const sidebar = document.getElementById("sessionSidebar");
          const dock = document.getElementById("dock");
          if (
            active instanceof HTMLElement &&
            (sidebar?.contains(active) || dock?.contains(active))
          ) {
            requestAnimationFrame(() => document.getElementById("composerInput")?.focus());
          }
        }
        syncSideToggle();
        toast(t(conversationOnly ? "shortcut.conversationOnly.entered" : "shortcut.conversationOnly.exited"));
      }
      syncSideToggle();

      document.getElementById("btnNewChat").addEventListener("click", () => {
        if (!window.__novaveiHost?.createSession) {
          toast(lang === "en" ? "New chat requires the desktop runtime." : "新建对话需要桌面运行时");
        }
      });

      document.getElementById("btnSettings").addEventListener("click", () => openOverlay("settings"));
      document.getElementById("openMcpFromSettings")?.addEventListener("click", () => openOverlay("mcp"));
      document.getElementById("openSkillsFromSettings")?.addEventListener("click", () => openOverlay("skills"));

      document.querySelectorAll("[data-close-overlay]").forEach((btn) => {
        btn.addEventListener("click", closeOverlays);
      });

      // Dock tools are intentionally opt-in. A fresh workspace starts with a
      // quiet reader and an empty tool shelf; users add only the panes they
      // need, while runtime actions can still explicitly bring a pane forward.
      const dockTabList = document.querySelector(".dock-tabs");
      const dockTabs = [...document.querySelectorAll(".dock .tab[data-pane]")];
      const dockPanels = [...document.querySelectorAll(".dock-pane[data-pane]")];
      const dock = document.getElementById("dock");
      const dockEmpty = document.getElementById("dockEmpty");
      const btnRevealProject = document.getElementById("btnRevealProject");
      const btnToggleDock = document.getElementById("btnToggleDock");
      const btnAddDockTool = document.getElementById("btnAddDockTool");
      const btnAddDockToolEmpty = document.getElementById("btnAddDockToolEmpty");
      const btnRemoveDockTool = document.getElementById("btnRemoveDockTool");
      const btnCloseDock = document.getElementById("btnCloseDock");
      const dockToolMenu = document.getElementById("dockToolMenu");
      const DOCK_TOOLS_KEY = "novavei.dockTools";
      const DOCK_ACTIVE_TOOL_KEY = "novavei.dockActiveTool";
      const DOCK_OPEN_KEY = "novavei.dockOpen";
      const knownDockTools = new Set(dockTabs.map((tab) => tab.dataset.pane).filter(Boolean));
      const dockLabelKeys = {
        run: "dock.labels.run",
        files: "dock.labels.files",
        git: "dock.labels.git",
        compare: "dock.labels.compare",
        browser: "dock.browser",
      };
      let dockTools = new Set();
      let activeDockTool;

      function dockToolLabel(name) {
        const key = dockLabelKeys[name];
        return key ? t(key) : name;
      }
      function readStoredDockTools() {
        try {
          const stored = JSON.parse(localStorage.getItem(DOCK_TOOLS_KEY) || "[]");
          return new Set(Array.isArray(stored) ? stored.filter((name) => knownDockTools.has(name)) : []);
        } catch {
          return new Set();
        }
      }
      function persistDockTools() {
        try {
          localStorage.setItem(DOCK_TOOLS_KEY, JSON.stringify([...dockTools]));
          if (activeDockTool) localStorage.setItem(DOCK_ACTIVE_TOOL_KEY, activeDockTool);
          else localStorage.removeItem(DOCK_ACTIVE_TOOL_KEY);
        } catch {}
      }
      function readStoredDockOpen() {
        try { return localStorage.getItem(DOCK_OPEN_KEY) === "true"; } catch { return false; }
      }
      function syncDockToggle() {
        const open = !workbench.classList.contains("dock-closed");
        const label = t(open ? "dock.toggle.close" : "dock.toggle.open");
        btnToggleDock?.setAttribute("aria-expanded", String(open));
        btnToggleDock?.setAttribute("aria-label", label);
        if (btnToggleDock) btnToggleDock.title = label;
      }
      function setDockOpen(open, options = {}) {
        workbench.classList.toggle("dock-closed", !open);
        if (options.persist !== false) {
          try { localStorage.setItem(DOCK_OPEN_KEY, String(open)); } catch {}
        }
        syncDockToggle();
      }
      function syncDockTools() {
        dockTabs.forEach((tab) => {
          const visible = dockTools.has(tab.dataset.pane);
          tab.hidden = !visible;
          tab.setAttribute("aria-hidden", String(!visible));
        });
        if (dockTabList) dockTabList.hidden = dockTools.size === 0;
        if (dockEmpty) dockEmpty.hidden = dockTools.size > 0;
        if (btnRemoveDockTool) btnRemoveDockTool.disabled = !activeDockTool;
      }
      function closeDockToolMenu(options = {}) {
        if (!dockToolMenu || dockToolMenu.hidden) return;
        dockToolMenu.hidden = true;
        btnAddDockTool?.setAttribute("aria-expanded", "false");
        if (options.restoreFocus) btnAddDockTool?.focus();
      }
      function openDockToolMenu() {
        if (!dockToolMenu) return;
        dockToolMenu.hidden = false;
        btnAddDockTool?.setAttribute("aria-expanded", "true");
        const first = dockToolMenu.querySelector("[data-dock-tool]");
        if (first instanceof HTMLElement) first.focus();
      }
      function showDockEmpty() {
        activeDockTool = undefined;
        dockTabs.forEach((tab) => {
          tab.classList.remove("on");
          tab.setAttribute("aria-selected", "false");
          tab.tabIndex = -1;
        });
        dockPanels.forEach((panel) => {
          panel.classList.remove("on");
          panel.hidden = true;
          panel.setAttribute("aria-hidden", "true");
        });
        syncDockTools();
      }
      function setDockPane(name, announce = true, options = {}) {
        const activeTab = dockTabs.find((tab) => tab.dataset.pane === name);
        if (!activeTab || !knownDockTools.has(name)) return;
        dockTools.add(name);
        activeDockTool = name;
        dockTabs.forEach((tab) => {
          const selected = tab === activeTab;
          tab.classList.toggle("on", selected);
          tab.setAttribute("aria-selected", String(selected));
          tab.tabIndex = selected ? 0 : -1;
        });
        dockPanels.forEach((panel) => {
          const selected = panel.dataset.pane === name;
          panel.classList.toggle("on", selected);
          panel.hidden = !selected;
          panel.setAttribute("aria-hidden", String(!selected));
        });
        syncDockTools();
        if (options.persist !== false) persistDockTools();
        if (options.open !== false) setDockOpen(true);
        window.dispatchEvent(new CustomEvent("novavei:dock-pane-activated", { detail: { pane: name } }));
        if (announce) toast((lang === "en" ? "Opened " : "已打开「") + dockToolLabel(name) + (lang === "en" ? "" : "」"));
      }
      function removeActiveDockTool() {
        if (!activeDockTool) return;
        const removed = activeDockTool;
        dockTools.delete(removed);
        const next = [...dockTools][0];
        if (next) setDockPane(next, false, { open: true, persist: false });
        else showDockEmpty();
        persistDockTools();
        const message = t("dock.removed").replace("{name}", dockToolLabel(removed));
        toast(message);
      }
      function closeDock() {
        closeDockToolMenu();
        const focusedInsideDock = document.activeElement instanceof HTMLElement && dock?.contains(document.activeElement);
        setDockOpen(false);
        if (focusedInsideDock) requestAnimationFrame(() => btnToggleDock?.focus());
      }
      function toggleDock() {
        const next = workbench.classList.contains("dock-closed");
        setDockOpen(next);
        toast(next ? (lang === "en" ? "Tools panel shown" : "已显示工具面板") : (lang === "en" ? "Tools panel hidden" : "已隐藏工具面板"));
      }
      dockTabs.forEach((tab) => {
        tab.addEventListener("click", () => setDockPane(tab.dataset.pane));
        tab.addEventListener("keydown", (event) => {
          const visibleTabs = dockTabs.filter((candidate) => !candidate.hidden);
          const currentIndex = visibleTabs.indexOf(tab);
          if (currentIndex < 0 || !visibleTabs.length) return;
          let nextIndex = currentIndex;
          if (event.key === "ArrowRight" || event.key === "ArrowDown") nextIndex = (currentIndex + 1) % visibleTabs.length;
          else if (event.key === "ArrowLeft" || event.key === "ArrowUp") nextIndex = (currentIndex - 1 + visibleTabs.length) % visibleTabs.length;
          else if (event.key === "Home") nextIndex = 0;
          else if (event.key === "End") nextIndex = visibleTabs.length - 1;
          else return;
          event.preventDefault();
          const nextTab = visibleTabs[nextIndex];
          nextTab.focus();
          nextTab.click();
        });
      });
      document.querySelectorAll("[data-dock-tool]").forEach((choice) => {
        choice.addEventListener("click", () => {
          if (choice instanceof HTMLButtonElement && choice.disabled) return;
          if (choice.getAttribute("aria-disabled") === "true") return;
          const name = choice.dataset.dockTool;
          if (!name) return;
          closeDockToolMenu();
          setDockPane(name);
        });
      });
      dockToolMenu?.addEventListener("keydown", (event) => {
        const choices = [...dockToolMenu.querySelectorAll("[data-dock-tool]")];
        const current = choices.indexOf(document.activeElement);
        let next = current;
        if (event.key === "ArrowDown") next = (current + 1 + choices.length) % choices.length;
        else if (event.key === "ArrowUp") next = (current - 1 + choices.length) % choices.length;
        else if (event.key === "Home") next = 0;
        else if (event.key === "End") next = choices.length - 1;
        else return;
        event.preventDefault();
        choices[next]?.focus();
      });
      btnAddDockTool?.addEventListener("click", () => {
        if (dockToolMenu?.hidden) openDockToolMenu();
        else closeDockToolMenu({ restoreFocus: true });
      });
      btnAddDockToolEmpty?.addEventListener("click", openDockToolMenu);
      btnRemoveDockTool?.addEventListener("click", removeActiveDockTool);
      document.addEventListener("pointerdown", (event) => {
        if (!dockToolMenu || dockToolMenu.hidden) return;
        const target = event.target;
        if (!(target instanceof Node)) return;
        if (!dockToolMenu.contains(target) && !btnAddDockTool?.contains(target)) closeDockToolMenu();
      });
      document.addEventListener("keydown", (event) => {
        if (event.key !== "Escape" || !dockToolMenu || dockToolMenu.hidden) return;
        event.preventDefault();
        closeDockToolMenu({ restoreFocus: true });
      });
      window.addEventListener("novavei:open-dock-tool", (event) => {
        const name = event instanceof CustomEvent ? event.detail?.pane : undefined;
        if (typeof name === "string") setDockPane(name);
      });
      dockTools = readStoredDockTools();
      let restoredActive;
      try { restoredActive = localStorage.getItem(DOCK_ACTIVE_TOOL_KEY); } catch {}
      if (typeof restoredActive === "string" && dockTools.has(restoredActive)) {
        setDockPane(restoredActive, false, { open: false, persist: false });
      } else if (dockTools.size) {
        setDockPane([...dockTools][0], false, { open: false, persist: false });
      } else {
        showDockEmpty();
      }
      setDockOpen(readStoredDockOpen(), { persist: false });
      window.addEventListener("novavei:language-changed", syncDockToggle);

      const DOCK_WIDTH_KEY = "novavei.dockWidth";
      const DOCK_WIDTH_MIN = 280;
      const DOCK_WIDTH_MAX = 720;
      const DOCK_WIDTH_DEFAULT = 360;
      const dockResizer = document.getElementById("dockResizer");
      let dockWidthPx = DOCK_WIDTH_DEFAULT;

      function clampDockWidth(value) {
        // Accept numbers or CSS lengths like "420px" / " 420px ".
        const numeric = typeof value === "number" ? value : parseFloat(String(value ?? "").trim());
        if (!Number.isFinite(numeric)) return dockWidthPx || DOCK_WIDTH_DEFAULT;
        const maxForViewport = Math.max(
          DOCK_WIDTH_MIN,
          Math.min(DOCK_WIDTH_MAX, Math.floor(window.innerWidth * 0.55)),
        );
        return Math.min(maxForViewport, Math.max(DOCK_WIDTH_MIN, Math.round(numeric)));
      }

      function applyDockWidth(width, options = {}) {
        const next = clampDockWidth(width);
        dockWidthPx = next;
        workbench.style.setProperty("--dock-width", next + "px");
        if (dockResizer) {
          dockResizer.setAttribute("aria-valuenow", String(next));
          dockResizer.setAttribute("aria-valuemin", String(DOCK_WIDTH_MIN));
          dockResizer.setAttribute("aria-valuemax", String(DOCK_WIDTH_MAX));
        }
        if (options.persist !== false) {
          try { localStorage.setItem(DOCK_WIDTH_KEY, String(next)); } catch {}
        }
        return next;
      }

      function readStoredDockWidth() {
        try {
          const raw = localStorage.getItem(DOCK_WIDTH_KEY);
          if (raw != null && raw !== "") return clampDockWidth(raw);
        } catch {}
        return DOCK_WIDTH_DEFAULT;
      }

      applyDockWidth(readStoredDockWidth(), { persist: false });

      if (dockResizer) {
        let drag = null;
        const onPointerMove = (event) => {
          if (!drag) return;
          const bounds = workbench.getBoundingClientRect();
          const next = bounds.right - event.clientX;
          applyDockWidth(next, { persist: false });
        };
        const endDrag = (event) => {
          if (!drag) return;
          if (event && drag.pointerId != null) {
            try { dockResizer.releasePointerCapture(drag.pointerId); } catch {}
          }
          workbench.classList.remove("is-resizing-dock");
          // Persist the live width we tracked during drag — do not re-parse
          // getComputedStyle (which used to feed "420px" into Number() → NaN → snap back).
          applyDockWidth(dockWidthPx, { persist: true });
          drag = null;
        };
        dockResizer.addEventListener("pointerdown", (event) => {
          if (workbench.classList.contains("dock-closed")) return;
          if (event.button != null && event.button !== 0) return;
          event.preventDefault();
          drag = { pointerId: event.pointerId };
          workbench.classList.add("is-resizing-dock");
          try { dockResizer.setPointerCapture(event.pointerId); } catch {}
        });
        dockResizer.addEventListener("pointermove", onPointerMove);
        dockResizer.addEventListener("pointerup", endDrag);
        dockResizer.addEventListener("pointercancel", endDrag);
        dockResizer.addEventListener("lostpointercapture", () => {
          if (!drag) return;
          workbench.classList.remove("is-resizing-dock");
          applyDockWidth(dockWidthPx, { persist: true });
          drag = null;
        });
        dockResizer.addEventListener("keydown", (event) => {
          if (workbench.classList.contains("dock-closed")) return;
          const step = event.shiftKey ? 32 : 16;
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            applyDockWidth(dockWidthPx + step);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            applyDockWidth(dockWidthPx - step);
          } else if (event.key === "Home") {
            event.preventDefault();
            applyDockWidth(DOCK_WIDTH_MAX);
          } else if (event.key === "End") {
            event.preventDefault();
            applyDockWidth(DOCK_WIDTH_MIN);
          }
        });
        window.addEventListener("resize", () => {
          applyDockWidth(dockWidthPx, { persist: true });
        });
      }

      const SIDEBAR_WIDTH_KEY = "novavei.sessionSidebarWidth";
      const SIDEBAR_WIDTH_MIN = 220;
      const SIDEBAR_WIDTH_MAX = 480;
      const SIDEBAR_WIDTH_DEFAULT = 280;
      const sidebarResizer = document.getElementById("sidebarResizer");
      let sidebarWidthPx = SIDEBAR_WIDTH_DEFAULT;

      function isSidebarResizeDisabled() {
        return (
          window.matchMedia("(max-width: 820px)").matches ||
          workbench.classList.contains("side-closed") ||
          workbench.classList.contains("conversation-only")
        );
      }

      function sidebarWidthMaxForViewport() {
        return Math.max(
          SIDEBAR_WIDTH_MIN,
          Math.min(SIDEBAR_WIDTH_MAX, Math.floor(window.innerWidth * 0.45)),
        );
      }

      function clampSidebarWidth(value) {
        const numeric = typeof value === "number" ? value : parseFloat(String(value ?? "").trim());
        if (!Number.isFinite(numeric)) return sidebarWidthPx || SIDEBAR_WIDTH_DEFAULT;
        const maxForViewport = sidebarWidthMaxForViewport();
        return Math.min(maxForViewport, Math.max(SIDEBAR_WIDTH_MIN, Math.round(numeric)));
      }

      function applySidebarWidth(width, options = {}) {
        const next = clampSidebarWidth(width);
        sidebarWidthPx = next;
        workbench.style.setProperty("--sessions-width", next + "px");
        if (sidebarResizer) {
          sidebarResizer.setAttribute("aria-valuenow", String(next));
          sidebarResizer.setAttribute("aria-valuemin", String(SIDEBAR_WIDTH_MIN));
          sidebarResizer.setAttribute(
            "aria-valuemax",
            String(sidebarWidthMaxForViewport()),
          );
        }
        if (options.persist !== false) {
          try { localStorage.setItem(SIDEBAR_WIDTH_KEY, String(next)); } catch {}
        }
        return next;
      }

      function readStoredSidebarWidth() {
        try {
          const raw = localStorage.getItem(SIDEBAR_WIDTH_KEY);
          if (raw != null && raw !== "") return clampSidebarWidth(raw);
        } catch {}
        return SIDEBAR_WIDTH_DEFAULT;
      }

      applySidebarWidth(readStoredSidebarWidth(), { persist: false });

      if (sidebarResizer) {
        let drag = null;
        const onPointerMove = (event) => {
          if (!drag) return;
          const bounds = workbench.getBoundingClientRect();
          const next = event.clientX - bounds.left;
          applySidebarWidth(next, { persist: false });
        };
        const endDrag = (event) => {
          if (!drag) return;
          if (event && drag.pointerId != null) {
            try { sidebarResizer.releasePointerCapture(drag.pointerId); } catch {}
          }
          workbench.classList.remove("is-resizing-sidebar");
          applySidebarWidth(sidebarWidthPx, { persist: true });
          drag = null;
        };
        sidebarResizer.addEventListener("pointerdown", (event) => {
          if (isSidebarResizeDisabled()) return;
          if (event.button != null && event.button !== 0) return;
          event.preventDefault();
          drag = { pointerId: event.pointerId };
          workbench.classList.add("is-resizing-sidebar");
          try { sidebarResizer.setPointerCapture(event.pointerId); } catch {}
        });
        sidebarResizer.addEventListener("pointermove", onPointerMove);
        sidebarResizer.addEventListener("pointerup", endDrag);
        sidebarResizer.addEventListener("pointercancel", endDrag);
        sidebarResizer.addEventListener("lostpointercapture", () => {
          if (!drag) return;
          workbench.classList.remove("is-resizing-sidebar");
          applySidebarWidth(sidebarWidthPx, { persist: true });
          drag = null;
        });
        sidebarResizer.addEventListener("dblclick", (event) => {
          if (isSidebarResizeDisabled()) return;
          event.preventDefault();
          applySidebarWidth(SIDEBAR_WIDTH_DEFAULT);
        });
        sidebarResizer.addEventListener("keydown", (event) => {
          if (isSidebarResizeDisabled()) return;
          const step = event.shiftKey ? 32 : 16;
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            applySidebarWidth(sidebarWidthPx - step);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            applySidebarWidth(sidebarWidthPx + step);
          } else if (event.key === "Home") {
            event.preventDefault();
            applySidebarWidth(SIDEBAR_WIDTH_MIN);
          } else if (event.key === "End") {
            event.preventDefault();
            applySidebarWidth(SIDEBAR_WIDTH_MAX);
          }
        });
        window.addEventListener("resize", () => {
          if (!window.matchMedia("(max-width: 820px)").matches) {
            applySidebarWidth(sidebarWidthPx, { persist: true });
          }
        });
      }

      let revealProjectBusy = false;

      function syncRevealProjectAvailability() {
        if (!btnRevealProject) return;
        const currentProject = document.querySelector(
          ".project-row[data-workdir][aria-current]",
        );
        const workdir = currentProject?.dataset?.workdir?.trim();
        const unregistered =
          currentProject?.dataset?.novaveiWorkspaceKind === "unregistered";
        const enabled = Boolean(workdir) && !unregistered && !revealProjectBusy;
        const label = unregistered
          ? t("project.revealRequiresRegistration")
          : workdir
            ? t("project.reveal")
            : t("project.noCurrent");
        btnRevealProject.disabled = !enabled;
        btnRevealProject.setAttribute("aria-disabled", String(!enabled));
        btnRevealProject.setAttribute("aria-label", label);
        btnRevealProject.title = label;
      }

      async function revealCurrentProjectInFileManager() {
        const currentProject = document.querySelector(
          ".project-row[data-workdir][aria-current]",
        );
        const workdir = currentProject?.dataset?.workdir?.trim();
        if (currentProject?.dataset?.novaveiWorkspaceKind === "unregistered") {
          syncRevealProjectAvailability();
          toast(t("project.revealRequiresRegistration"));
          return;
        }
        if (!workdir) {
          syncRevealProjectAvailability();
          toast(t("project.noCurrent"));
          return;
        }
        const invoke = window.__TAURI__?.core?.invoke;
        if (typeof invoke !== "function") {
          toast(t("project.revealDesktopOnly"));
          return;
        }
        revealProjectBusy = true;
        syncRevealProjectAvailability();
        try {
          await invoke("workspace_reveal", { workdir });
          toast(t("project.revealed"));
        } catch {
          // The native command intentionally keeps platform/path details out
          // of the WebView, so offer one concise recovery route here as well.
          toast(t("project.revealFailed"));
        } finally {
          revealProjectBusy = false;
          syncRevealProjectAvailability();
        }
      }

      btnRevealProject?.addEventListener("click", () => {
        void revealCurrentProjectInFileManager();
      });
      window.addEventListener(
        "novavei:current-project-changed",
        syncRevealProjectAvailability,
      );
      window.addEventListener("novavei:language-changed", syncRevealProjectAvailability);
      syncRevealProjectAvailability();
      btnToggleDock?.addEventListener("click", toggleDock);
      btnCloseDock?.addEventListener("click", () => {
        closeDock();
        toast(lang === "en" ? "Tools panel hidden" : "已隐藏工具面板");
      });
      document.getElementById("btnToggleSide").addEventListener("click", () => {
        if (window.matchMedia("(max-width: 820px)").matches) {
          workbench.classList.toggle("side-open");
        } else {
          workbench.classList.toggle("side-closed");
        }
        syncSideToggle();
      });


      // Permission picker lives in src/runtime/permission-picker.ts
      // Model labels are hydrated from native provider settings by runtime/dom.ts.
      const models = ["5.6 Sol", "5.6 Terra", "5.6 Luna", "5.5", "5.4", "5.4 Mini", "5.2"];
      const reasoningLevels = ["关闭", "最少", "轻度", "中", "高", "极高", "最高"];
      const reasoningDescriptions = [
        "不请求额外推理，适合只需快速响应的任务。",
        "使用最少推理，兼顾速度与基本判断。",
        "更快完成，适合简单问答。",
        "在速度与推理之间取得平衡。",
        "适合多步骤分析与编辑任务。",
        "更深入的推理会消耗更多额度。",
        "最大化思考深度，适合复杂任务。",
      ];
      let modelIdx = 0;
      let reasoningIdx = 5;
      const modelBtn = document.getElementById("btnModel");
      const modelPopover = document.getElementById("modelPopover");
      const modelControl = modelBtn.closest(".model-control");
      const reasoningSlider = document.getElementById("reasoningSlider");
      const advancedModelsBtn = document.getElementById("btnAdvancedModels");
      const modelOptions = document.getElementById("modelOptions");

      function syncModelPicker() {
        const selectedOption = document.querySelector(`.model-option[data-model="${modelIdx}"]`);
        const selectedModelLabel = selectedOption?.dataset.modelLabel || models[modelIdx] || "未选择模型";
        document.getElementById("modelPickerName").textContent = selectedModelLabel;
        document.getElementById("modelPickerReasoning").textContent = reasoningLevels[reasoningIdx];
        document.getElementById("reasoningValue").textContent = reasoningLevels[reasoningIdx];
        document.getElementById("reasoningHelper").textContent = reasoningDescriptions[reasoningIdx];
        reasoningSlider.value = String(reasoningIdx);
        reasoningSlider.setAttribute("aria-valuetext", reasoningLevels[reasoningIdx]);
        reasoningSlider.style.setProperty("--range-fill", `${(reasoningIdx / (reasoningLevels.length - 1)) * 100}%`);
        document.querySelectorAll(".reasoning-step").forEach((step) => {
          step.classList.toggle("on", Number(step.dataset.reasoning) === reasoningIdx);
        });
        document.querySelectorAll(".model-option").forEach((option) => {
          const selected = Number(option.dataset.model) === modelIdx;
          const optionLabel = option.dataset.modelLabel || models[Number(option.dataset.model)] || option.textContent?.trim() || "Model";
          option.classList.toggle("on", selected);
          option.setAttribute("aria-pressed", String(selected));
          option.replaceChildren(document.createTextNode(optionLabel));
          if (selected) {
            const check = document.createElementNS("http://www.w3.org/2000/svg", "svg");
            check.setAttribute("class", "ico");
            check.setAttribute("viewBox", "0 0 24 24");
            check.setAttribute("aria-hidden", "true");
            const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
            path.setAttribute("d", "m5 12 4 4L19 6");
            check.appendChild(path);
            option.appendChild(check);
          }
        });
      }

      function closeModelPopover(restoreFocus = false) {
        modelPopover.classList.remove("show");
        modelBtn.setAttribute("aria-expanded", "false");
        if (restoreFocus) modelBtn.focus();
      }

      function setReasoning(value) {
        reasoningIdx = Math.max(0, Math.min(reasoningLevels.length - 1, Number(value)));
        syncModelPicker();
      }

      modelBtn.addEventListener("click", () => {
        const willOpen = !modelPopover.classList.contains("show");
        if (!willOpen) {
          closeModelPopover();
          return;
        }
        modelPopover.classList.add("show");
        modelBtn.setAttribute("aria-expanded", "true");
        requestAnimationFrame(() => reasoningSlider.focus());
      });
      reasoningSlider.addEventListener("input", () => setReasoning(reasoningSlider.value));
      document.querySelectorAll(".reasoning-step").forEach((step) => {
        step.addEventListener("click", () => setReasoning(step.dataset.reasoning));
      });
      function selectModelOption(option, restoreFocus = true) {
        const nextIndex = Number(option?.dataset?.model);
        if (!Number.isInteger(nextIndex) || nextIndex < 0) return;
        modelIdx = nextIndex;
        syncModelPicker();
        closeModelPopover(restoreFocus);
        toast("模型：" + (option.dataset.modelLabel || models[modelIdx] || "未选择模型"));
      }
      advancedModelsBtn.addEventListener("click", () => {
        const willOpen = modelOptions.hidden;
        modelOptions.hidden = !willOpen;
        advancedModelsBtn.setAttribute("aria-expanded", String(willOpen));
      });
      modelOptions.addEventListener("click", (event) => {
        const option = event.target instanceof Element ? event.target.closest(".model-option") : null;
        if (!(option instanceof HTMLElement) || !modelOptions.contains(option)) return;
        selectModelOption(option, true);
      });
      window.addEventListener("novavei:model-options-rendered", (event) => {
        const requested = Number(event?.detail?.selectedIndex);
        const options = [...modelOptions.querySelectorAll(".model-option")];
        const selected = options.find((option) => option.classList.contains("on"));
        const selectedIndex = Number(selected?.dataset.model);
        modelIdx = options.some((option) => Number(option.dataset.model) === requested)
          ? requested
          : Number.isInteger(selectedIndex) && selectedIndex >= 0
            ? selectedIndex
            : 0;
        syncModelPicker();
      });
      document.addEventListener("pointerdown", (event) => {
        if (modelPopover.classList.contains("show") && !modelControl.contains(event.target)) closeModelPopover();
      });
      syncModelPicker();

      const runDetailsBtn = document.getElementById("btnRunDetails");
      const composerRunSteps = document.getElementById("composerRunSteps");
      runDetailsBtn.addEventListener("click", () => {
        const willOpen = composerRunSteps.hidden;
        composerRunSteps.hidden = !willOpen;
        runDetailsBtn.setAttribute("aria-expanded", String(willOpen));
      });

      // Composer send
      const form = document.getElementById("composerForm");
      const input = document.getElementById("composerInput");
      form.addEventListener("submit", (e) => {
        e.preventDefault();
        // The capture-phase bridge below owns real Pi submission. If that
        // bridge is unavailable, fail closed instead of fabricating a reply.
        if (!window.__novaveiPiRuntime?.submit) toast("Pi 运行时未连接，消息未发送");
      });

      // Generic toast buttons (static data-toast + i18n data-toast-key)
      document.querySelectorAll("[data-toast], [data-toast-key]").forEach((el) => {
        el.addEventListener("click", () => {
          if (el.dataset.toastKey) toast(t(el.dataset.toastKey));
          else if (el.dataset.toast) toast(el.dataset.toast);
        });
      });

      document.getElementById("btnCommand")?.addEventListener("click", openSearchPalette);
      document.getElementById("btnTheme")?.addEventListener("click", () => {
        setTheme(document.documentElement.dataset.theme === "light" ? "dark" : "light");
      });

      document.getElementById("sessionSearch")?.addEventListener("input", (e) => {
        const q = e.target.value.trim().toLowerCase();
        let sessionMatches = 0;
        let projectMatches = 0;
        document.querySelectorAll(".session-group .session").forEach((session) => {
          const matches = !q || session.textContent.toLowerCase().includes(q);
          session.hidden = !matches;
          if (matches) sessionMatches += 1;
        });
        document.querySelectorAll(".session-group").forEach((group) => {
          group.hidden = Boolean(q) && !group.querySelector(".session:not([hidden])");
        });
        document.querySelectorAll(".project-folder").forEach((folder) => {
          const project = folder.querySelector(".project-row");
          const projectMatchesQuery = !q || project.textContent.toLowerCase().includes(q);
          let childMatches = 0;
          folder.querySelectorAll(".project-conversations .session").forEach((session) => {
            const matches = !q || projectMatchesQuery || session.textContent.toLowerCase().includes(q);
            session.hidden = !matches;
            if (matches) {
              childMatches += 1;
              sessionMatches += 1;
            }
          });
          const matches = projectMatchesQuery || childMatches > 0;
          folder.hidden = !matches;
          if (matches) projectMatches += 1;
          setProjectExpanded(project, q ? matches : project.dataset.expanded === "true");
        });
        const projectSection = document.getElementById("projectSection");
        if (projectSection) {
          projectSection.hidden = Boolean(q) && !projectSection.querySelector(".project-folder:not([hidden])");
        }
        const otherWorkspaces = document.getElementById("novaveiOtherWorkspacesSection");
        if (otherWorkspaces) {
          otherWorkspaces.hidden = Boolean(q)
            ? !otherWorkspaces.querySelector(".project-folder:not([hidden])")
            : !otherWorkspaces.querySelector(".project-folder");
        }
        document.getElementById("sidebarEmpty").hidden = !q || sessionMatches + projectMatches > 0;
      });

      // Settings nav — horizontal slide between sections
      const settingsStage = document.getElementById("settingsStage");
      const settingsNavButtons = [...document.querySelectorAll(".settings-nav button")];
      const settingsKeys = settingsNavButtons.map((btn) => btn.dataset.settings);
      let settingsIndex = Math.max(0, settingsKeys.indexOf(
        document.querySelector(".settings-panel.on")?.dataset.settings || settingsKeys[0]
      ));
      let settingsAnimating = false;

      function clearSettingsAnimClasses(panel) {
        panel.classList.remove("is-leaving", "is-entering", "dir-left", "dir-right", "from-left", "from-right");
        panel.style.transform = "";
        panel.style.opacity = "";
      }

      function goToSettings(key, { animate = true } = {}) {
        const nextIdx = settingsKeys.indexOf(key);
        if (nextIdx < 0 || nextIdx === settingsIndex || settingsAnimating) return;

        const prevIdx = settingsIndex;
        const forward = nextIdx > prevIdx;
        const prevPanel = settingsStage.querySelector(`.settings-panel[data-settings="${settingsKeys[prevIdx]}"]`);
        const nextPanel = settingsStage.querySelector(`.settings-panel[data-settings="${settingsKeys[nextIdx]}"]`);
        if (!prevPanel || !nextPanel) return;

        settingsNavButtons.forEach((btn) => btn.classList.toggle("on", btn.dataset.settings === key));
        settingsIndex = nextIdx;

        const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        if (!animate || reduceMotion) {
          settingsStage.querySelectorAll(".settings-panel").forEach((panel) => {
            clearSettingsAnimClasses(panel);
            panel.classList.toggle("on", panel === nextPanel);
          });
          return;
        }

        settingsAnimating = true;
        const stageHeight = Math.max(prevPanel.offsetHeight, nextPanel.scrollHeight, 280);
        settingsStage.style.minHeight = stageHeight + "px";

        clearSettingsAnimClasses(prevPanel);
        clearSettingsAnimClasses(nextPanel);
        prevPanel.classList.remove("on");
        prevPanel.classList.add("is-leaving", forward ? "dir-left" : "dir-right");
        nextPanel.classList.add("on", "is-entering", forward ? "from-right" : "from-left");

        let settled = false;
        const finish = () => {
          if (settled) return;
          settled = true;
          clearSettingsAnimClasses(prevPanel);
          clearSettingsAnimClasses(nextPanel);
          prevPanel.classList.remove("on", "is-leaving", "is-entering");
          nextPanel.classList.remove("is-leaving", "is-entering");
          nextPanel.classList.add("on");
          settingsStage.style.minHeight = "";
          settingsAnimating = false;
        };
        nextPanel.addEventListener("animationend", finish, { once: true });
        setTimeout(finish, 360);
      }

      settingsNavButtons.forEach((btn) => {
        btn.addEventListener("click", () => goToSettings(btn.dataset.settings));
      });

      // Swipe left / right on settings content
      let swipeStartX = 0;
      let swipeStartY = 0;
      let swipeActive = false;
      let swipeLocked = false;
      settingsStage.addEventListener("pointerdown", (e) => {
        if (settingsAnimating || e.button) return;
        if (e.target.closest("button, input, select, textarea, a, label")) return;
        swipeActive = true;
        swipeLocked = false;
        swipeStartX = e.clientX;
        swipeStartY = e.clientY;
        settingsStage.setPointerCapture?.(e.pointerId);
      });
      settingsStage.addEventListener("pointermove", (e) => {
        if (!swipeActive || settingsAnimating) return;
        const dx = e.clientX - swipeStartX;
        const dy = e.clientY - swipeStartY;
        if (!swipeLocked) {
          if (Math.abs(dx) < 10 && Math.abs(dy) < 10) return;
          if (Math.abs(dy) > Math.abs(dx)) {
            swipeActive = false;
            return;
          }
          swipeLocked = true;
          settingsStage.classList.add("dragging");
        }
        const current = settingsStage.querySelector(".settings-panel.on");
        if (current) {
          const resistance = (settingsIndex === 0 && dx > 0) || (settingsIndex === settingsKeys.length - 1 && dx < 0) ? 0.28 : 0.9;
          current.style.transform = `translateX(${dx * resistance}px)`;
          current.style.opacity = String(Math.max(0.55, 1 - Math.abs(dx) / 480));
        }
      });
      function endSettingsSwipe(e) {
        if (!swipeActive) return;
        swipeActive = false;
        settingsStage.classList.remove("dragging");
        const dx = e.clientX - swipeStartX;
        const current = settingsStage.querySelector(".settings-panel.on");
        if (current) {
          current.style.transform = "";
          current.style.opacity = "";
        }
        if (!swipeLocked || Math.abs(dx) < 64) return;
        if (dx < 0 && settingsIndex < settingsKeys.length - 1) {
          goToSettings(settingsKeys[settingsIndex + 1]);
        } else if (dx > 0 && settingsIndex > 0) {
          goToSettings(settingsKeys[settingsIndex - 1]);
        }
      }
      settingsStage.addEventListener("pointerup", endSettingsSwipe);
      settingsStage.addEventListener("pointercancel", endSettingsSwipe);

      function setToolsTab(name) {
        document.querySelectorAll("[data-tools-tab]").forEach((btn) => {
          const selected = btn.dataset.toolsTab === name;
          btn.classList.toggle("on", selected);
          btn.setAttribute("aria-selected", String(selected));
        });
        document.querySelectorAll("[data-tools-panel]").forEach((panel) => {
          panel.classList.toggle("on", panel.dataset.toolsPanel === name);
        });
      }
      document.querySelectorAll("[data-tools-tab]").forEach((btn) => {
        btn.addEventListener("click", () => setToolsTab(btn.dataset.toolsTab));
      });

      function setSystemTab(name) {
        document.querySelectorAll("[data-system-tab]").forEach((btn) => {
          const selected = btn.dataset.systemTab === name;
          btn.classList.toggle("on", selected);
          btn.setAttribute("aria-selected", String(selected));
        });
        document.querySelectorAll("[data-system-panel]").forEach((panel) => {
          panel.classList.toggle("on", panel.dataset.systemPanel === name);
        });
      }
      document.querySelectorAll("[data-system-tab]").forEach((btn) => {
        btn.addEventListener("click", () => setSystemTab(btn.dataset.systemTab));
      });

      function setMemoryTab(name) {
        document.querySelectorAll("[data-memory-tab]").forEach((btn) => {
          const selected = btn.dataset.memoryTab === name;
          btn.classList.toggle("on", selected);
          btn.setAttribute("aria-selected", String(selected));
          btn.tabIndex = selected ? 0 : -1;
        });
        document.querySelectorAll("[data-memory-panel]").forEach((panel) => {
          const selected = panel.dataset.memoryPanel === name;
          panel.classList.toggle("on", selected);
          panel.hidden = !selected;
        });
      }
      const memoryTabs = [...document.querySelectorAll("[data-memory-tab]")];
      memoryTabs.forEach((btn) => {
        btn.addEventListener("click", () => setMemoryTab(btn.dataset.memoryTab));
        btn.addEventListener("keydown", (event) => {
          const current = memoryTabs.indexOf(btn);
          if (current < 0) return;
          let next = current;
          if (event.key === "ArrowRight") next = (current + 1) % memoryTabs.length;
          else if (event.key === "ArrowLeft") next = (current - 1 + memoryTabs.length) % memoryTabs.length;
          else if (event.key === "Home") next = 0;
          else if (event.key === "End") next = memoryTabs.length - 1;
          else return;
          event.preventDefault();
          const target = memoryTabs[next];
          setMemoryTab(target.dataset.memoryTab);
          target.focus();
        });
      });

      function setCouncilTab(name) {
        document.querySelectorAll("[data-council-tab]").forEach((btn) => {
          const selected = btn.dataset.councilTab === name;
          btn.classList.toggle("on", selected);
          btn.setAttribute("aria-selected", String(selected));
        });
        document.querySelectorAll("[data-council-panel]").forEach((panel) => {
          panel.classList.toggle("on", panel.dataset.councilPanel === name);
        });
      }
      document.querySelectorAll("[data-council-tab]").forEach((btn) => {
        btn.addEventListener("click", () => setCouncilTab(btn.dataset.councilTab));
      });

      document.querySelectorAll("[data-lang-opt]").forEach((btn) => {
        btn.addEventListener("click", () => {
          setLanguage(btn.dataset.langOpt, true);
          syncLangSeg();
        });
      });
      document.querySelectorAll("[data-theme-opt]").forEach((btn) => {
        btn.addEventListener("click", () => {
          const value = btn.dataset.themeOpt;
          themePref = value;
          if (value === "system") {
            const prefersLight = systemThemeQuery?.matches ?? false;
            setTheme(prefersLight ? "light" : "dark", { pref: "system" });
            return;
          }
          setTheme(value, { pref: value });
        });
      });
      syncLangSeg();
      syncThemeSeg();
      document.querySelectorAll("[data-ui-scale]").forEach((btn) => {
        btn.addEventListener("click", () => setUiScale(btn.dataset.uiScale, true));
      });
      document.querySelectorAll("[data-user-color]").forEach((btn) => {
        btn.addEventListener("click", () => applyUserAccent(btn.dataset.userColor, true));
      });
      document.getElementById("userColorPicker")?.addEventListener("input", (e) => {
        applyUserAccent(e.target.value, false);
      });
      document.getElementById("userColorPicker")?.addEventListener("change", (e) => {
        applyUserAccent(e.target.value, true);
      });
      setUiScale(100, false);
      applyUserAccent("#0A84FF", false);
      const UI_FONT_STACKS = {
  "system": "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI Variable Text\", \"Segoe UI\", system-ui, \"PingFang SC\", \"Microsoft YaHei\", sans-serif",
  "inter": "Inter, \"Segoe UI\", system-ui, \"PingFang SC\", \"Microsoft YaHei\", sans-serif",
  "segoe": "\"Segoe UI Variable Text\", \"Segoe UI\", system-ui, sans-serif",
  "pingfang": "\"PingFang SC\", \"Hiragino Sans GB\", \"Microsoft YaHei\", sans-serif",
  "yahei": "\"Microsoft YaHei UI\", \"Microsoft YaHei\", \"PingFang SC\", sans-serif",
  "noto": "\"Noto Sans SC\", \"PingFang SC\", \"Microsoft YaHei\", sans-serif",
  "source": "\"Source Han Sans SC\", \"Noto Sans SC\", \"Microsoft YaHei\", sans-serif"
};
      const CODE_FONT_STACKS = {
  "system": "\"Cascadia Code\", \"SF Mono\", Consolas, \"Segoe UI Mono\", monospace",
  "cascadia": "\"Cascadia Code\", \"Cascadia Mono\", Consolas, monospace",
  "consolas": "Consolas, \"Courier New\", monospace",
  "fira": "\"Fira Code\", \"Cascadia Code\", Consolas, monospace",
  "jetbrains": "\"JetBrains Mono\", \"Cascadia Code\", Consolas, monospace",
  "sfmono": "\"SF Mono\", \"Menlo\", Monaco, Consolas, monospace",
  "sourcecode": "\"Source Code Pro\", \"Cascadia Code\", Consolas, monospace"
};
      let uiFontKey = "system";
      let codeFontKey = "system";
      function setUiFont(key, announce) {
        uiFontKey = key || "system";
        const stack = UI_FONT_STACKS[uiFontKey] || UI_FONT_STACKS.system;
        document.documentElement.style.setProperty("--font", stack);
        document.body.style.fontFamily = "var(--font)";
        const sel = document.getElementById("settingsUiFont");
        if (sel) sel.value = uiFontKey;
        if (announce) {
          const label = sel?.selectedOptions?.[0]?.textContent || uiFontKey;
          toast((lang === "en" ? "UI font: " : "界面字体：") + label);
        }
      }
      function setCodeFont(key, announce) {
        codeFontKey = key || "system";
        const stack = CODE_FONT_STACKS[codeFontKey] || CODE_FONT_STACKS.system;
        document.documentElement.style.setProperty("--mono", stack);
        const sel = document.getElementById("settingsCodeFont");
        if (sel) sel.value = codeFontKey;
        if (announce) {
          const label = sel?.selectedOptions?.[0]?.textContent || codeFontKey;
          toast((lang === "en" ? "Code font: " : "代码字体：") + label);
        }
      }
      document.getElementById("settingsUiFont")?.addEventListener("change", (e) => {
        setUiFont(e.target.value, true);
      });
      document.getElementById("settingsCodeFont")?.addEventListener("change", (e) => {
        setCodeFont(e.target.value, true);
      });
      setUiFont("system", false);
      setCodeFont("system", false);

      // Provider settings are native-backed. The old static cards intentionally
      // stay out of the runtime so a browser preview can never look connected.
      const providerList = document.getElementById("providerList");
      const providerImportPreview = document.getElementById("providerImportPreview");
      const providerEditorModal = document.getElementById("providerEditorModal");
      const providerEditorView = document.getElementById("providerEditorView");
      const PROVIDER_DRAFT = Symbol("providerDraft");
      const PROVIDER_DISCOVERY_PLACEHOLDER = "custom-model";
      // Keep this in lockstep with the native import allowlist. Unknown paths
      // are omitted rather than rendering arbitrary imported URL fragments.
      const PROVIDER_IMPORT_PUBLIC_API_ROOTS = new Set([
        "/",
        "/v1",
        "/v1beta",
        "/v1alpha",
        "/api/v1",
        "/openai/v1",
        "/compatible-mode/v1",
        "/api/paas/v4",
      ]);
      let providerState = [];
      let providerLoadSerial = 0;
      let providerImportToken = "";
      let providerImportItems = [];
      let providerImportBusy = false;
      let providerImportReturnFocus = null;
      let providerEditorReturnFocus = null;
      let providerEditorRecord = null;
      let providerEditorModels = [];
      let providerEditorHeaderRows = [];
      let providerEditorFetchedMeta = new Map();
      let providerEditorSavedConnection = null;
      let providerEditorBusy = false;
      let providerEditorSerial = 0;

      function providerEditorField(name) {
        return providerEditorView?.querySelector(`[data-provider-editor-field="${name}"]`) || null;
      }

      function providerEditorSetTab(tab) {
        if (!providerEditorView) return;
        for (const button of providerEditorView.querySelectorAll("[data-provider-editor-tab]")) {
          button.classList.toggle("on", button.getAttribute("data-provider-editor-tab") === tab);
        }
        for (const panel of providerEditorView.querySelectorAll("[data-provider-editor-panel]")) {
          panel.classList.toggle("on", panel.getAttribute("data-provider-editor-panel") === tab);
        }
      }

      function providerEditorSetStatus(message, state = "") {
        const status = providerEditorView?.querySelector("[data-provider-editor-status]");
        if (!(status instanceof HTMLElement)) return;
        status.textContent = message || "";
        status.setAttribute("role", state === "error" ? "alert" : "status");
        status.setAttribute("aria-live", state === "error" ? "assertive" : "polite");
        if (state) status.dataset.state = state;
        else delete status.dataset.state;
      }

      function providerEditorReadCustomHeadersFromRecord(record) {
        const rows = [];
        const source = record?.customHeaders ?? record?.headers;
        if (Array.isArray(source)) {
          for (const item of source) {
            const object = providerObject(item) || {};
            const key = providerString(object, "key", "name", "header");
            if (!key) continue;
            rows.push({
              key,
              value: "",
              valueConfigured: object.valueConfigured === true || Boolean(providerString(object, "value")),
              cleared: false,
            });
          }
        } else {
          const object = providerObject(source);
          if (object) {
            for (const [key, value] of Object.entries(object)) {
              if (!key) continue;
              rows.push({
                key,
                value: "",
                valueConfigured: value === true || (typeof value === "string" && Boolean(value)) || (providerObject(value)?.valueConfigured === true),
                cleared: false,
              });
            }
          }
        }
        return rows;
      }

      function providerEditorReadModelsFromRecord(record) {
        const ids = providerModelIds(record);
        const active = new Set(
          (Array.isArray(record?.activeModels) ? record.activeModels : ids)
            .map((item) => (typeof item === "string" ? item.trim() : ""))
            .filter(Boolean),
        );
        const defaultModel = providerString(record, "defaultModel", "default_model") || ids[0] || "";
        const models = [];
        const sourceModels = Array.isArray(record?.models) ? record.models : [];
        for (const id of ids) {
          if (id === PROVIDER_DISCOVERY_PLACEHOLDER && record?.modelDiscoveryPlaceholder) continue;
          const raw = sourceModels.find((item) => (typeof item === "string" ? item : providerString(item || {}, "id", "modelId", "model_id")) === id);
          const object = providerObject(raw);
          models.push({
            id,
            label: object ? providerString(object, "label", "displayName", "name") : "",
            enabled: active.has(id) || active.size === 0,
            isDefault: id === defaultModel,
          });
        }
        if (models.length && !models.some((model) => model.isDefault)) {
          const firstEnabled = models.find((model) => model.enabled) || models[0];
          firstEnabled.isDefault = true;
        }
        return models;
      }

      function providerEditorRenderModels() {
        const list = document.getElementById("providerModelCatalogueList");
        const search = document.getElementById("providerModelSearch");
        if (!(list instanceof HTMLElement)) return;
        list.replaceChildren();
        const query = (search instanceof HTMLInputElement ? search.value : "").trim().toLowerCase();
        const visible = providerEditorModels.filter((model) => {
          if (!query) return true;
          return model.id.toLowerCase().includes(query) || (model.label || "").toLowerCase().includes(query);
        });
        if (!visible.length) {
          const empty = document.createElement("div");
          empty.className = "provider-empty";
          empty.setAttribute("data-i18n", "settings.providers.noModels");
          empty.textContent = t("settings.providers.noModels");
          list.appendChild(empty);
          return;
        }
        for (const model of visible) {
          const row = document.createElement("div");
          row.className = "provider-model-row";
          row.setAttribute("role", "listitem");
          const enabled = document.createElement("input");
          enabled.type = "checkbox";
          enabled.className = "provider-model-enabled";
          enabled.checked = model.enabled !== false;
          enabled.title = t("settings.providers.enabled");
          enabled.addEventListener("change", () => {
            model.enabled = enabled.checked;
            if (!model.enabled && model.isDefault) {
              model.isDefault = false;
              const next = providerEditorModels.find((item) => item.enabled);
              if (next) next.isDefault = true;
              providerEditorRenderModels();
            }
          });
          const label = document.createElement("span");
          label.className = "mono";
          label.textContent = model.label && model.label !== model.id ? `${model.id} · ${model.label}` : model.id;
          const defaultRadio = document.createElement("input");
          defaultRadio.type = "radio";
          defaultRadio.name = "providerEditorDefaultModel";
          defaultRadio.checked = Boolean(model.isDefault);
          defaultRadio.title = t("settings.providers.defaultModel");
          defaultRadio.addEventListener("change", () => {
            if (!defaultRadio.checked) return;
            for (const item of providerEditorModels) item.isDefault = item === model;
            model.enabled = true;
            providerEditorRenderModels();
          });
          const remove = document.createElement("button");
          remove.type = "button";
          remove.className = "btn ghost";
          remove.textContent = "×";
          remove.setAttribute("aria-label", t("settings.providers.headerRemove"));
          remove.addEventListener("click", () => {
            providerEditorModels = providerEditorModels.filter((item) => item !== model);
            if (providerEditorModels.length && !providerEditorModels.some((item) => item.isDefault)) {
              const next = providerEditorModels.find((item) => item.enabled) || providerEditorModels[0];
              next.isDefault = true;
            }
            providerEditorRenderModels();
          });
          row.append(enabled, label, defaultRadio, remove);
          list.appendChild(row);
        }
      }

      function providerEditorRenderHeaders() {
        const list = providerEditorView?.querySelector("[data-provider-header-list]");
        if (!(list instanceof HTMLElement)) return;
        list.replaceChildren();
        if (!providerEditorHeaderRows.length) {
          const empty = document.createElement("div");
          empty.className = "provider-empty";
          empty.setAttribute("data-i18n", "settings.providers.headerNote");
          empty.textContent = t("settings.providers.headerNote");
          list.appendChild(empty);
          return;
        }
        for (const row of providerEditorHeaderRows) {
          const wrap = document.createElement("div");
          wrap.className = "provider-header-row";
          const key = document.createElement("input");
          key.type = "text";
          key.autocomplete = "off";
          key.placeholder = t("settings.providers.headerName");
          key.value = row.key || "";
          key.addEventListener("input", () => { row.key = key.value; });
          const value = document.createElement("input");
          value.type = "password";
          value.autocomplete = "new-password";
          value.placeholder = row.valueConfigured && !row.cleared
            ? t("settings.providers.headerConfigured")
            : t("settings.providers.headerValue");
          value.value = row.value || "";
          value.addEventListener("input", () => {
            row.value = value.value;
            if (value.value.trim()) row.cleared = false;
          });
          const clear = document.createElement("button");
          clear.type = "button";
          clear.className = "btn ghost";
          clear.setAttribute("data-i18n", "settings.providers.headerClear");
          clear.textContent = t("settings.providers.headerClear");
          clear.disabled = !(row.valueConfigured || row.value);
          clear.addEventListener("click", () => {
            row.value = "";
            row.cleared = true;
            row.valueConfigured = false;
            value.value = "";
            value.placeholder = t("settings.providers.headerValue");
            clear.disabled = true;
          });
          const remove = document.createElement("button");
          remove.type = "button";
          remove.className = "btn ghost";
          remove.setAttribute("data-i18n", "settings.providers.headerRemove");
          remove.textContent = t("settings.providers.headerRemove");
          remove.addEventListener("click", () => {
            providerEditorHeaderRows = providerEditorHeaderRows.filter((item) => item !== row);
            providerEditorRenderHeaders();
          });
          wrap.append(key, value, clear, remove);
          list.appendChild(wrap);
        }
      }

      function providerEditorSyncFormatVisibility() {
        const type = providerEditorField("type");
        const wrap = providerEditorView?.querySelector("[data-provider-editor-request-format-wrap]");
        if (wrap instanceof HTMLElement && type instanceof HTMLSelectElement) {
          wrap.hidden = type.value !== "codex";
        }
      }

      function providerEditorConnectionIdentity() {
        const type = providerEditorField("type");
        const baseUrl = providerEditorField("baseUrl");
        const requestFormat = providerEditorField("requestFormat");
        return {
          type: type instanceof HTMLSelectElement ? type.value : "codex",
          baseUrl: baseUrl instanceof HTMLInputElement ? baseUrl.value.trim() : "",
          requestFormat: requestFormat instanceof HTMLSelectElement ? requestFormat.value : "openai-responses",
        };
      }

      function providerEditorConnectionChanged() {
        if (!providerEditorSavedConnection) return Boolean(providerEditorRecord?.[PROVIDER_DRAFT]);
        const current = providerEditorConnectionIdentity();
        return current.type !== providerEditorSavedConnection.type
          || current.baseUrl !== providerEditorSavedConnection.baseUrl
          || (current.type === "codex" && current.requestFormat !== providerEditorSavedConnection.requestFormat);
      }

      function providerEditorSyncReentryWarning() {
        const note = providerEditorView?.querySelector("[data-provider-editor-reentry]");
        const apiKey = providerEditorField("apiKey");
        const changed = providerEditorConnectionChanged();
        if (note instanceof HTMLElement) note.hidden = !changed || Boolean(providerEditorRecord?.[PROVIDER_DRAFT]);
        if (apiKey instanceof HTMLInputElement && changed && !providerEditorRecord?.[PROVIDER_DRAFT]) {
          apiKey.placeholder = t("settings.providers.reentryKey");
        } else if (apiKey instanceof HTMLInputElement) {
          apiKey.placeholder = providerConfigured(providerEditorRecord || {})
            ? t("settings.providers.apiKey")
            : "sk-…";
        }
      }

      function providerEditorPopulate(record) {
        providerEditorRecord = record;
        providerEditorFetchedMeta = new Map();
        providerEditorModels = providerEditorReadModelsFromRecord(record);
        providerEditorHeaderRows = providerEditorReadCustomHeadersFromRecord(record);
        providerEditorSavedConnection = record[PROVIDER_DRAFT] ? null : {
          type: providerType(record),
          baseUrl: providerBaseUrl(record),
          requestFormat: providerString(record, "requestFormat", "request_format") || "openai-responses",
        };
        const title = document.getElementById("providerEditorViewTitle");
        if (title) {
          title.textContent = providerString(record, "name", "label") || providerId(record) || t("settings.providers.editorTitle");
        }
        const name = providerEditorField("name");
        const id = providerEditorField("id");
        const type = providerEditorField("type");
        const baseUrl = providerEditorField("baseUrl");
        const apiKey = providerEditorField("apiKey");
        const requestFormat = providerEditorField("requestFormat");
        const enabled = providerEditorField("enabled");
        const defaultCheck = providerEditorField("default");
        const systemProxy = providerEditorField("systemProxy");
        const requestProxy = providerEditorField("requestProxy");
        const promptCaching = providerEditorField("promptCachingEnabled");
        if (name instanceof HTMLInputElement) name.value = providerString(record, "name", "label") || providerId(record);
        if (id instanceof HTMLInputElement) {
          id.value = providerId(record);
          id.disabled = !record[PROVIDER_DRAFT];
        }
        if (type instanceof HTMLSelectElement) type.value = providerType(record);
        if (baseUrl instanceof HTMLInputElement) baseUrl.value = providerBaseUrl(record);
        if (apiKey instanceof HTMLInputElement) apiKey.value = "";
        if (requestFormat instanceof HTMLSelectElement) {
          requestFormat.value = providerString(record, "requestFormat", "request_format") || "openai-responses";
        }
        if (enabled instanceof HTMLInputElement) enabled.checked = record.enabled !== false;
        if (defaultCheck instanceof HTMLInputElement) {
          defaultCheck.checked = record.default === true || record.isDefault === true;
        }
        const useProxy = record.useSystemProxy === true || record.systemProxy === true;
        if (systemProxy instanceof HTMLInputElement) systemProxy.checked = useProxy;
        if (requestProxy instanceof HTMLInputElement) requestProxy.checked = useProxy;
        if (promptCaching instanceof HTMLInputElement) {
          promptCaching.checked = record.promptCachingEnabled !== false;
        }
        const removeButton = providerEditorView?.querySelector('[data-provider-editor-action="remove"]');
        if (removeButton instanceof HTMLButtonElement) removeButton.hidden = Boolean(record[PROVIDER_DRAFT]);
        providerEditorSyncFormatVisibility();
        providerEditorSyncReentryWarning();
        providerEditorRenderModels();
        providerEditorRenderHeaders();
        providerEditorSetStatus("");
        providerEditorSetTab("general");
      }

      function openProviderEditorView(record, trigger) {
        if (!providerEditorView || !providerEditorModal) return;
        providerEditorReturnFocus = trigger instanceof HTMLElement ? trigger : null;
        providerEditorView.dataset.providerId = providerId(record);
        providerEditorPopulate(record);
        providerEditorModal.hidden = false;
        providerEditorModal.classList.add("show");
        window.requestAnimationFrame(() => providerEditorField("name")?.focus());
      }

      function closeProviderEditorView(options = {}) {
        if (!providerEditorView || !providerEditorModal) return;
        const discardDraft = options.discardDraft !== false;
        if (discardDraft && providerEditorRecord?.[PROVIDER_DRAFT]) {
          providerState = providerState.filter((item) => item !== providerEditorRecord);
        }
        providerEditorRecord = null;
        providerEditorModels = [];
        providerEditorHeaderRows = [];
        providerEditorFetchedMeta = new Map();
        providerEditorSavedConnection = null;
        providerEditorModal.classList.remove("show");
        providerEditorModal.hidden = true;
        delete providerEditorView.dataset.providerId;
        renderProviders();
        providerEditorReturnFocus?.focus({ preventScroll: true });
        providerEditorReturnFocus = null;
      }

      function providerEditorCollectCustomHeaders() {
        const headers = [];
        for (const row of providerEditorHeaderRows) {
          const key = String(row.key || "").trim();
          if (!key) continue;
          const value = String(row.value || "").trim();
          if (value) {
            headers.push({ key, value });
            continue;
          }
          if (row.valueConfigured && !row.cleared) {
            headers.push({ key, valueConfigured: true });
          }
        }
        return headers;
      }

      function providerEditorReadDraft(options = {}) {
        const requireModels = options.requireModels !== false;
        const nameField = providerEditorField("name");
        const idField = providerEditorField("id");
        const typeField = providerEditorField("type");
        const baseUrlField = providerEditorField("baseUrl");
        const apiKeyField = providerEditorField("apiKey");
        const requestFormatField = providerEditorField("requestFormat");
        const enabledField = providerEditorField("enabled");
        const defaultField = providerEditorField("default");
        const systemProxyField = providerEditorField("systemProxy");
        const requestProxyField = providerEditorField("requestProxy");
        const promptCachingField = providerEditorField("promptCachingEnabled");
        const name = nameField instanceof HTMLInputElement ? nameField.value.trim() : "";
        const id = idField instanceof HTMLInputElement ? idField.value.trim() : "";
        const type = typeField instanceof HTMLSelectElement ? typeField.value : "codex";
        const baseUrl = baseUrlField instanceof HTMLInputElement ? baseUrlField.value.trim() : "";
        const enteredKey = apiKeyField instanceof HTMLInputElement ? apiKeyField.value.trim() : "";
        const modelIds = providerEditorModels.map((model) => model.id).filter(Boolean);
        let parsedUrl = null;
        try { parsedUrl = new URL(baseUrl); } catch { parsedUrl = null; }
        if (!name || !/^[A-Za-z0-9._-]{1,128}$/.test(id) || !parsedUrl || !["http:", "https:"].includes(parsedUrl.protocol) || parsedUrl.username || parsedUrl.password || parsedUrl.search || parsedUrl.hash) {
          toast(t("settings.providers.invalid"));
          return null;
        }
        if (requireModels && !modelIds.length) {
          toast(t("settings.providers.invalid"));
          return null;
        }
        const enabledModels = providerEditorModels.filter((model) => model.enabled !== false).map((model) => model.id);
        const activeModels = enabledModels.length ? enabledModels : modelIds.slice(0, 1);
        const defaultModel = (providerEditorModels.find((model) => model.isDefault && model.enabled !== false)
          || providerEditorModels.find((model) => model.enabled !== false)
          || providerEditorModels[0])?.id || activeModels[0] || modelIds[0] || PROVIDER_DISCOVERY_PLACEHOLDER;
        const useSystemProxy = Boolean(
          (systemProxyField instanceof HTMLInputElement && systemProxyField.checked)
          || (requestProxyField instanceof HTMLInputElement && requestProxyField.checked),
        );
        const draft = {
          id,
          name,
          type,
          baseUrl,
          models: modelIds.length
            ? modelIds.map((modelId) => {
              const local = providerEditorModels.find((model) => model.id === modelId);
              const fetched = providerEditorFetchedMeta.get(modelId);
              const old = (Array.isArray(providerEditorRecord?.models) ? providerEditorRecord.models : [])
                .find((item) => (typeof item === "string" ? item : providerString(item || {}, "id", "modelId", "model_id")) === modelId);
              if ((old && typeof old === "object") || fetched || (local?.label && local.label !== modelId)) {
                return {
                  ...(old && typeof old === "object" ? old : {}),
                  ...(fetched || {}),
                  id: modelId,
                  ...(local?.label && local.label !== modelId ? { label: local.label } : {}),
                };
              }
              return modelId;
            })
            : [PROVIDER_DISCOVERY_PLACEHOLDER],
          activeModels: activeModels.length ? activeModels : [defaultModel],
          defaultModel,
          enabled: !(enabledField instanceof HTMLInputElement) || enabledField.checked,
          useSystemProxy,
          promptCachingEnabled: !(promptCachingField instanceof HTMLInputElement) || promptCachingField.checked,
          default: Boolean(defaultField instanceof HTMLInputElement && defaultField.checked),
          isDefault: Boolean(defaultField instanceof HTMLInputElement && defaultField.checked),
        };
        if (!modelIds.length) draft.modelDiscoveryPlaceholder = true;
        if (type === "codex") {
          draft.requestFormat = requestFormatField instanceof HTMLSelectElement
            ? requestFormatField.value
            : "openai-responses";
          draft.protocol = draft.requestFormat;
        } else {
          draft.protocol = type === "gemini" ? "google-generative-ai" : "anthropic-messages";
        }
        const customHeaders = providerEditorCollectCustomHeaders();
        if (customHeaders.length) draft.customHeaders = customHeaders;
        if (enteredKey) draft.apiKey = enteredKey;
        else if (providerEditorRecord && !providerEditorRecord[PROVIDER_DRAFT] && !providerEditorConnectionChanged() && providerConfigured(providerEditorRecord)) {
          draft.apiKeyConfigured = true;
        }
        return draft;
      }

      async function provider_draft_prepare(provider) {
        return providerInvoke("provider_draft_prepare", { provider });
      }

      async function provider_draft_test(draftToken) {
        return providerInvoke("provider_draft_test", { draftToken });
      }

      async function provider_draft_models_fetch(draftToken) {
        return providerInvoke("provider_draft_models_fetch", { draftToken });
      }

      async function providerEditorPrepareDraftToken() {
        const draft = providerEditorReadDraft({ requireModels: false });
        if (!draft) return null;
        if (!draft.models?.length) draft.models = [PROVIDER_DISCOVERY_PLACEHOLDER];
        try {
          const result = await provider_draft_prepare(draft);
          const token = result?.draftToken || result?.draft_token;
          if (!token) throw new Error("missing draft token");
          return token;
        } catch (error) {
          providerEditorSetStatus(
            fillProviderMessage("settings.providers.draftPrepareFailed", { error: providerErrorText(error) }),
            "error",
          );
          return null;
        }
      }

      async function providerEditorCatalogueCheck() {
        if (providerEditorBusy) return;
        const serial = ++providerEditorSerial;
        providerEditorBusy = true;
        providerEditorSetStatus(t("settings.providers.fetchModelsLoading"), "loading");
        try {
          const token = await providerEditorPrepareDraftToken();
          if (!token || serial !== providerEditorSerial) return;
          const result = await provider_draft_test(token);
          if (serial !== providerEditorSerial) return;
          if (result && result.ok === false) throw new Error(result.error || result.message || "catalogue check failed");
          const detail = [
            result?.status ? `HTTP ${result.status}` : "OK",
            Number.isFinite(Number(result?.latencyMs)) ? `${Math.round(Number(result.latencyMs))} ms` : "",
            Number.isFinite(Number(result?.modelCount)) ? `${result.modelCount} ${t("settings.providers.models")}` : "",
          ].filter(Boolean).join(" · ");
          providerEditorSetStatus(
            fillProviderMessage("settings.providers.catalogueCheckOk", { detail }),
            "success",
          );
        } catch (error) {
          if (serial !== providerEditorSerial) return;
          const raw = providerErrorText(error);
          providerEditorSetStatus(
            fillProviderMessage("settings.providers.catalogueCheckFailed", {
              error: /provider_draft|unknown command|not allowed/i.test(raw)
                ? t("settings.providers.testUnavailable")
                : raw,
            }),
            "error",
          );
        } finally {
          if (serial === providerEditorSerial) providerEditorBusy = false;
        }
      }

      async function providerEditorFetchModels() {
        if (providerEditorBusy) return;
        const serial = ++providerEditorSerial;
        providerEditorBusy = true;
        providerEditorSetStatus(t("settings.providers.fetchModelsLoading"), "loading");
        try {
          const token = await providerEditorPrepareDraftToken();
          if (!token || serial !== providerEditorSerial) return;
          const result = await provider_draft_models_fetch(token);
          if (serial !== providerEditorSerial) return;
          if (result?.availability === "unsupported") {
            providerEditorSetStatus(t("settings.providers.fetchModelsUnsupported"));
            return;
          }
          const fetched = fetchedProviderModels(result?.models);
          if (!fetched.length) {
            providerEditorSetStatus(t("settings.providers.fetchModelsEmpty"));
            return;
          }
          const existing = new Map(providerEditorModels.map((model) => [model.id, model]));
          for (const model of fetched) {
            providerEditorFetchedMeta.set(model.id, model);
            const previous = existing.get(model.id);
            if (previous) {
              previous.label = model.label || previous.label;
            } else {
              existing.set(model.id, {
                id: model.id,
                label: model.label || "",
                enabled: true,
                isDefault: false,
              });
            }
          }
          providerEditorModels = [...existing.values()];
          if (!providerEditorModels.some((model) => model.isDefault)) {
            const first = providerEditorModels.find((model) => model.enabled) || providerEditorModels[0];
            if (first) first.isDefault = true;
          }
          providerEditorRenderModels();
          providerEditorSetStatus(
            result?.truncated === true
              ? fillProviderMessage("settings.providers.fetchModelsTruncated", { count: fetched.length })
              : fillProviderMessage("settings.providers.fetchModelsSuccess", { count: fetched.length }),
            "success",
          );
        } catch (error) {
          if (serial !== providerEditorSerial) return;
          providerEditorSetStatus(
            fillProviderMessage("settings.providers.fetchModelsFailed", {
              error: providerFetchErrorText(error),
            }),
            "error",
          );
        } finally {
          if (serial === providerEditorSerial) providerEditorBusy = false;
        }
      }

      async function providerEditorSave() {
        if (!providerEditorRecord || providerEditorBusy) return;
        const draft = providerEditorReadDraft({ requireModels: true });
        if (!draft) return;
        if (providerEditorConnectionChanged() && !draft.apiKey && !providerMayBeKeyless(draft)) {
          toast(t("settings.providers.reentryKey"));
          providerEditorSyncReentryWarning();
          return;
        }
        const duplicate = providerState.some((item) => item !== providerEditorRecord && providerId(item) === draft.id);
        if (duplicate) {
          toast(lang === "en" ? "Provider ID already exists." : "供应商标识已存在。");
          return;
        }
        const next = { ...providerEditorRecord, ...draft };
        if (!draft.apiKey) delete next.apiKey;
        for (const key of providerConfiguredFlagKeys(next)) {
          if (draft.apiKey) delete next[key];
        }
        if (draft.apiKeyConfigured) next.apiKeyConfigured = true;
        if (!draft.customHeaders) delete next.customHeaders;
        if (!draft.modelDiscoveryPlaceholder) delete next.modelDiscoveryPlaceholder;
        delete next[PROVIDER_DRAFT];
        if (next.default || next.isDefault) {
          for (const item of providerState) {
            if (item !== providerEditorRecord) {
              item.default = false;
              item.isDefault = false;
            }
          }
          next.default = true;
          next.isDefault = true;
        }
        const index = providerState.indexOf(providerEditorRecord);
        if (index < 0) return;
        const previous = providerState[index];
        providerState = providerState.slice();
        providerState[index] = next;
        providerEditorBusy = true;
        try {
          await saveProviderState(providerState);
          toast(fillProviderMessage("settings.providers.saved", { name: draft.name }));
          closeProviderEditorView({ discardDraft: false });
        } catch (error) {
          providerState[index] = previous;
          providerEditorRecord = previous;
          await appError(error, lang === "en" ? "Could not save provider" : "保存供应商失败");
        } finally {
          providerEditorBusy = false;
        }
      }

      async function providerEditorRemove() {
        if (!providerEditorRecord || providerEditorRecord[PROVIDER_DRAFT]) return;
        const nameText = providerString(providerEditorRecord, "name", "label") || providerId(providerEditorRecord);
        if (!(await appConfirm({
          title: lang === "en" ? "Delete provider" : "删除供应商",
          message: lang === "en" ? `Delete provider “${nameText}”?` : `确定删除供应商“${nameText}”？`,
          confirmLabel: lang === "en" ? "Delete" : "删除",
          cancelLabel: lang === "en" ? "Cancel" : "取消",
          danger: true,
        }))) return;
        const id = providerId(providerEditorRecord);
        try {
          await saveProviderState(
            providerState.filter((item) => item !== providerEditorRecord),
            [id],
          );
          toast(fillProviderMessage("settings.providers.removed", { name: nameText }));
          closeProviderEditorView({ discardDraft: false });
        } catch (error) {
          await appError(error, lang === "en" ? "Could not delete provider" : "删除供应商失败");
        }
      }

      function providerEditorAddModel() {
        const input = document.getElementById("providerModelManualId");
        const id = input instanceof HTMLInputElement ? input.value.trim() : "";
        if (!id) return;
        if (providerEditorModels.some((model) => model.id === id)) {
          if (input instanceof HTMLInputElement) input.value = "";
          return;
        }
        providerEditorModels.push({
          id,
          label: "",
          enabled: true,
          isDefault: providerEditorModels.length === 0,
        });
        if (input instanceof HTMLInputElement) input.value = "";
        providerEditorRenderModels();
      }

      function bindProviderEditorView() {
        if (!providerEditorView || providerEditorView.dataset.bound === "1") return;
        providerEditorView.dataset.bound = "1";
        providerEditorView.querySelector("[data-provider-editor-close]")?.addEventListener("click", () => {
          closeProviderEditorView();
        });
        providerEditorModal?.addEventListener("click", (event) => {
          if (event.target === providerEditorModal) closeProviderEditorView();
        });
        for (const button of providerEditorView.querySelectorAll("[data-provider-editor-tab]")) {
          button.addEventListener("click", () => {
            providerEditorSetTab(button.getAttribute("data-provider-editor-tab") || "general");
          });
        }
        for (const field of ["type", "baseUrl", "requestFormat"]) {
          const control = providerEditorField(field);
          control?.addEventListener("input", () => {
            providerEditorSyncFormatVisibility();
            providerEditorSyncReentryWarning();
          });
          control?.addEventListener("change", () => {
            providerEditorSyncFormatVisibility();
            providerEditorSyncReentryWarning();
          });
        }
        const systemProxy = providerEditorField("systemProxy");
        const requestProxy = providerEditorField("requestProxy");
        systemProxy?.addEventListener("change", () => {
          if (requestProxy instanceof HTMLInputElement && systemProxy instanceof HTMLInputElement) {
            requestProxy.checked = systemProxy.checked;
          }
        });
        requestProxy?.addEventListener("change", () => {
          if (systemProxy instanceof HTMLInputElement && requestProxy instanceof HTMLInputElement) {
            systemProxy.checked = requestProxy.checked;
          }
        });
        document.getElementById("providerModelSearch")?.addEventListener("input", () => providerEditorRenderModels());
        providerEditorView.querySelector('[data-provider-editor-action="add-model"]')?.addEventListener("click", () => providerEditorAddModel());
        providerEditorView.querySelector('[data-provider-editor-action="add-header"]')?.addEventListener("click", () => {
          providerEditorHeaderRows.push({ key: "", value: "", valueConfigured: false, cleared: false });
          providerEditorRenderHeaders();
        });
        providerEditorView.querySelector('[data-provider-editor-action="catalogue-check"]')?.addEventListener("click", () => {
          void providerEditorCatalogueCheck();
        });
        providerEditorView.querySelector('[data-provider-editor-action="fetch-models"]')?.addEventListener("click", () => {
          void providerEditorFetchModels();
        });
        providerEditorView.querySelector('[data-provider-editor-action="cancel"]')?.addEventListener("click", () => {
          closeProviderEditorView();
        });
        providerEditorView.querySelector('[data-provider-editor-action="remove"]')?.addEventListener("click", () => {
          void providerEditorRemove();
        });
        providerEditorView.querySelector('[data-provider-editor-action="save"]')?.addEventListener("click", () => {
          void providerEditorSave();
        });
      }

      bindProviderEditorView();

      function providerInvoke(command, args = {}) {
        const core = window.__TAURI__?.core;
        if (!core?.invoke) throw new Error(t("settings.providers.unavailable"));
        return core.invoke(command, args);
      }

      function providerObject(value) {
        return value && typeof value === "object" && !Array.isArray(value) ? value : null;
      }

      function providerRecords(value) {
        if (Array.isArray(value)) return value.filter((item) => providerObject(item)).map((item) => ({ ...item }));
        const object = providerObject(value);
        if (!object) return [];
        const nested = object.providers ?? object.items ?? object.customProviders;
        if (nested !== undefined) return providerRecords(nested);
        if (typeof object.id === "string" || typeof object.providerId === "string") return [{ ...object }];
        return Object.entries(object).flatMap(([id, item]) => {
          const record = providerObject(item);
          return record ? [{ ...record, id: record.id || record.providerId || id }] : [];
        });
      }

      function providerString(record, ...keys) {
        for (const key of keys) {
          const value = record?.[key];
          if (typeof value === "string" && value.trim()) return value.trim();
        }
        return "";
      }

      function providerId(record) {
        return providerString(record, "id", "providerId", "provider_id");
      }

      function providerType(record) {
        const explicit = providerString(record, "type").toLowerCase();
        if (["codex", "claude_code", "gemini"].includes(explicit)) return explicit;
        const hint = `${providerString(record, "protocol", "api", "requestFormat")} ${providerId(record)}`.toLowerCase();
        if (hint.includes("anthropic") || hint.includes("claude")) return "claude_code";
        if (hint.includes("gemini") || hint.includes("google")) return "gemini";
        return "codex";
      }

      function providerBaseUrl(record) {
        return providerString(record, "baseUrl", "base_url", "endpoint", "url");
      }

      function providerModelIds(record) {
        const values = [];
        const models = Array.isArray(record?.models) ? record.models : [];
        for (const item of models) {
          const value = typeof item === "string" ? item : providerString(item || {}, "id", "modelId", "model_id", "name");
          if (value && !values.includes(value)) values.push(value);
        }
        for (const item of Array.isArray(record?.activeModels) ? record.activeModels : []) {
          if (typeof item === "string" && item.trim() && !values.includes(item.trim())) values.push(item.trim());
        }
        const direct = providerString(record, "defaultModel", "default_model", "model");
        if (direct && !values.includes(direct)) values.unshift(direct);
        return values;
      }

      function providerModelIdsFromText(value) {
        return String(value || "")
          .split(/[\n,]/)
          .map((item) => item.trim())
          .filter(Boolean)
          .filter((item, index, list) => list.indexOf(item) === index);
      }

      // A freshly added provider needs one editable model to satisfy the
      // existing save contract before native discovery is available. Keep this
      // explicit marker until discovery replaces the temporary value, so the
      // placeholder can never become the automatic default model.
      function hasProviderDiscoveryPlaceholder(record, modelIds) {
        return record?.modelDiscoveryPlaceholder === true
          && modelIds.length === 1
          && modelIds[0] === PROVIDER_DISCOVERY_PLACEHOLDER;
      }

      // Model discovery is a native-only request. Treat its response as a
      // constrained model record, not arbitrary provider configuration: only
      // Pi's supported id/label/context fields enter the draft for review.
      function fetchedProviderModels(value) {
        const output = [];
        const seen = new Set();
        for (const item of Array.isArray(value) ? value : []) {
          const record = providerObject(item);
          const id = (typeof item === "string" ? item : providerString(record || {}, "id", "modelId", "model_id", "name")).trim();
          if (!id || id.length > 256 || seen.has(id) || output.length >= 256) continue;
          seen.add(id);
          const label = record ? providerString(record, "label", "displayName", "display_name", "name") : "";
          const model = label && label !== id
            ? { id, label: label.slice(0, 256) }
            : { id };
          const contextWindow = Number(record?.contextWindow);
          if (Number.isSafeInteger(contextWindow) && contextWindow > 0 && contextWindow <= 0xFFFF_FFFF) {
            model.contextWindow = contextWindow;
          }
          const maxOutputToken = Number(record?.maxOutputToken);
          if (Number.isSafeInteger(maxOutputToken) && maxOutputToken > 0 && maxOutputToken <= 0xFFFF_FFFF) {
            model.maxOutputToken = maxOutputToken;
          }
          output.push(model);
        }
        return output;
      }

      function providerFetchErrorText(error) {
        const raw = providerErrorText(error);
        if (/provider_models_fetch|unknown command|not allowed/i.test(raw)) {
          return t("settings.providers.fetchModelsUnavailable");
        }
        return raw;
      }

      function providerConfigured(record) {
        const secretKeys = new Set(["apikey", "api_key", "token", "access_token", "secret", "password", "key"]);
        const walk = (value, inHeaders = false) => {
          if (Array.isArray(value)) return value.some((item) => walk(item, inHeaders));
          const object = providerObject(value);
          if (!object) return false;
          return Object.entries(object).some(([key, item]) => {
            const lowered = key.toLowerCase();
            if (lowered.endsWith("configured") && (secretKeys.has(lowered.slice(0, -10)) || lowered === "valueconfigured")) {
              return item === true;
            }
            if (!inHeaders && secretKeys.has(lowered) && typeof item === "string") return Boolean(item.trim());
            return walk(item, inHeaders || ["headers", "customheaders", "custom_headers"].includes(lowered));
          });
        };
        return walk(record);
      }

      function providerLoopback(record) {
        try {
          const url = new URL(providerBaseUrl(record));
          return ["localhost", "127.0.0.1", "::1"].includes(url.hostname.toLowerCase());
        } catch {
          return false;
        }
      }

      function providerMayBeKeyless(record) {
        return providerLoopback(record) || record?.requiresApiKey === false || providerString(record, "auth", "authentication").toLowerCase() === "none";
      }

      function providerDisplayUrl(record) {
        const raw = providerBaseUrl(record).replace(/[?#].*$/, "");
        try {
          const url = new URL(raw);
          return `${url.host}${url.pathname.replace(/\/$/, "")}` || url.host;
        } catch {
          return raw || t("settings.providers.newMeta");
        }
      }

      function providerProtocolLabel(record) {
        const type = providerType(record);
        return t(type === "claude_code" ? "settings.providers.typeClaude" : type === "gemini" ? "settings.providers.typeGemini" : "settings.providers.typeCodex");
      }

      function providerInitials(record) {
        const name = providerString(record, "name", "label") || providerId(record) || "N";
        const chars = Array.from(name.replace(/[^\p{L}\p{N}]+/gu, "")).slice(0, 2);
        return (chars.join("") || "N").toUpperCase();
      }

      function providerStatus(record) {
        if (record[PROVIDER_DRAFT]) return { key: "settings.providers.statusDraft", className: "wait" };
        if (record.__testStatus === "testing") return { key: "settings.providers.statusTesting", className: "wait" };
        if (record.__testStatus === "failed") return { key: "settings.providers.statusFailed", className: "warn" };
        if (record.__testStatus === "ok") return { key: "settings.providers.statusOk", className: "" };
        if (providerConfigured(record)) return { key: "settings.providers.statusConfigured", className: "" };
        if (providerMayBeKeyless(record)) return { key: "settings.providers.statusLocal", className: "wait" };
        return { key: "settings.providers.statusNeedKey", className: "warn" };
      }

      function providerTag(text, mono = false) {
        const tag = document.createElement("span");
        tag.className = `tag${mono ? " mono" : ""}`;
        tag.textContent = text;
        return tag;
      }

      function editorField(form, field, labelKey, control, wide = false) {
        const wrap = document.createElement("div");
        wrap.className = `provider-editor-field${wide ? " wide" : ""}`;
        const id = `provider-editor-${field}-${Math.random().toString(36).slice(2, 8)}`;
        const label = document.createElement("label");
        label.htmlFor = id;
        label.setAttribute("data-i18n", labelKey);
        label.textContent = t(labelKey);
        control.id = id;
        control.dataset.providerField = field;
        control.setAttribute("aria-label", t(labelKey));
        wrap.append(label, control);
        form.appendChild(wrap);
        return wrap;
      }

      function providerSelectOptions(select, options) {
        for (const [value, key] of options) {
          const option = document.createElement("option");
          option.value = value;
          option.setAttribute("data-i18n", key);
          option.textContent = t(key);
          select.appendChild(option);
        }
      }

      function createProviderEditor(record, card) {
        const form = document.createElement("form");
        form.className = "provider-editor";
        form.hidden = true;
        form.noValidate = true;

        const name = document.createElement("input");
        name.type = "text";
        name.value = providerString(record, "name", "label") || providerId(record);
        name.autocomplete = "off";
        editorField(form, "name", "settings.providers.name", name);

        const id = document.createElement("input");
        id.type = "text";
        id.value = providerId(record);
        id.pattern = "[A-Za-z0-9._-]+";
        id.disabled = !record[PROVIDER_DRAFT];
        id.autocomplete = "off";
        editorField(form, "id", "settings.providers.id", id);

        const type = document.createElement("select");
        providerSelectOptions(type, [
          ["codex", "settings.providers.typeCodex"],
          ["claude_code", "settings.providers.typeClaude"],
          ["gemini", "settings.providers.typeGemini"],
        ]);
        type.value = providerType(record);
        editorField(form, "type", "settings.providers.type", type);

        const baseUrl = document.createElement("input");
        baseUrl.type = "url";
        baseUrl.value = providerBaseUrl(record);
        baseUrl.placeholder = "https://api.example.com/v1";
        baseUrl.autocomplete = "url";
        editorField(form, "baseUrl", "settings.providers.baseUrl", baseUrl);

        const models = document.createElement("textarea");
        models.value = providerModelIds(record).join("\n");
        models.placeholder = "gpt-4.1-mini";
        models.spellcheck = false;
        const modelsField = editorField(form, "models", "settings.providers.modelIds", models, true);
        const fetchedModelMetadata = new Map();
        let invalidateModelFetch = () => {};

        const apiKey = document.createElement("input");
        apiKey.type = "password";
        apiKey.value = "";
        apiKey.placeholder = providerConfigured(record) ? t("settings.providers.apiKey") : "sk-…";
        apiKey.autocomplete = "new-password";
        editorField(form, "apiKey", "settings.providers.apiKey", apiKey, true);

        const requestFormat = document.createElement("select");
        providerSelectOptions(requestFormat, [
          ["openai-responses", "settings.providers.formatResponses"],
          ["openai-completions", "settings.providers.formatCompletions"],
        ]);
        requestFormat.value = providerString(record, "requestFormat", "request_format") || "openai-responses";
        const requestFormatField = editorField(form, "requestFormat", "settings.providers.requestFormat", requestFormat);

        const checks = document.createElement("div");
        checks.className = "provider-editor-checks";
        const defaultLabel = document.createElement("label");
        const defaultCheck = document.createElement("input");
        defaultCheck.type = "checkbox";
        defaultCheck.dataset.providerField = "default";
        defaultCheck.checked = record.default === true || record.isDefault === true;
        const defaultText = document.createElement("span");
        defaultText.setAttribute("data-i18n", "settings.providers.default");
        defaultText.textContent = t("settings.providers.default");
        defaultLabel.append(defaultCheck, defaultText);
        checks.appendChild(defaultLabel);
        const proxyLabel = document.createElement("label");
        const proxyCheck = document.createElement("input");
        proxyCheck.type = "checkbox";
        proxyCheck.dataset.providerField = "systemProxy";
        proxyCheck.checked = record.useSystemProxy === true || record.systemProxy === true;
        const proxyText = document.createElement("span");
        proxyText.setAttribute("data-i18n", "settings.providers.systemProxy");
        proxyText.textContent = t("settings.providers.systemProxy");
        proxyLabel.append(proxyCheck, proxyText);
        checks.appendChild(proxyLabel);
        form.appendChild(checks);

        // Fetching deliberately targets the persisted provider only. A new key,
        // endpoint, or protocol is not sent across this IPC boundary; save that
        // connection change first, then fetch with native-held credentials.
        if (!record[PROVIDER_DRAFT]) {
          const fetchActions = document.createElement("div");
          fetchActions.className = "provider-model-actions";
          const fetchModels = document.createElement("button");
          fetchModels.type = "button";
          fetchModels.className = "btn";
          fetchModels.setAttribute("data-i18n", "settings.providers.fetchModels");
          fetchModels.textContent = t("settings.providers.fetchModels");
          const fetchStatus = document.createElement("p");
          fetchStatus.className = "provider-model-fetch-status";
          fetchStatus.setAttribute("role", "status");
          fetchStatus.setAttribute("aria-live", "polite");
          fetchActions.append(fetchModels, fetchStatus);
          modelsField.appendChild(fetchActions);

          const savedConnection = {
            type: type.value,
            baseUrl: baseUrl.value.trim(),
            requestFormat: requestFormat.value,
            useSystemProxy: Boolean(proxyCheck.checked),
          };
          let modelFetchSerial = 0;
          let fetchingModels = false;
          const connectionMatchesSaved = () => (
            type.value === savedConnection.type
            && baseUrl.value.trim() === savedConnection.baseUrl
            && (type.value !== "codex" || requestFormat.value === savedConnection.requestFormat)
            && Boolean(proxyCheck.checked) === savedConnection.useSystemProxy
            && !apiKey.value.trim()
          );
          const setFetchStatus = (message, state = "") => {
            fetchStatus.textContent = message;
            fetchStatus.setAttribute("role", state === "error" ? "alert" : "status");
            fetchStatus.setAttribute("aria-live", state === "error" ? "assertive" : "polite");
            if (state) fetchStatus.dataset.state = state;
            else delete fetchStatus.dataset.state;
          };
          const syncModelFetchAvailability = () => {
            const usable = connectionMatchesSaved();
            fetchModels.disabled = fetchingModels || !usable;
            if (!fetchingModels && !usable) {
              setFetchStatus(t("settings.providers.fetchModelsSavedOnly"), "blocked");
            } else if (!fetchingModels && fetchStatus.dataset.state === "blocked") {
              setFetchStatus("");
            }
          };
          const invalidate = () => {
            modelFetchSerial += 1;
          };
          const onSavedConnectionInput = () => {
            if (fetchingModels && !connectionMatchesSaved()) {
              invalidate();
              fetchingModels = false;
              fetchModels.removeAttribute("aria-busy");
            }
            syncModelFetchAvailability();
          };
          invalidateModelFetch = () => {
            invalidate();
            fetchingModels = false;
            fetchModels.removeAttribute("aria-busy");
            syncModelFetchAvailability();
          };
          [type, baseUrl, requestFormat, apiKey, proxyCheck].forEach((control) => {
            control.addEventListener("input", onSavedConnectionInput);
            control.addEventListener("change", onSavedConnectionInput);
          });
          fetchModels.addEventListener("click", () => {
            void (async () => {
              if (!connectionMatchesSaved()) {
                syncModelFetchAvailability();
                return;
              }
              const serial = ++modelFetchSerial;
              fetchingModels = true;
              fetchModels.disabled = true;
              fetchModels.setAttribute("aria-busy", "true");
              setFetchStatus(t("settings.providers.fetchModelsLoading"), "loading");
              try {
                const result = await providerInvoke("provider_models_fetch", { providerId: providerId(record) });
                if (serial !== modelFetchSerial || !form.isConnected) return;
                if (result?.availability === "unsupported") {
                  setFetchStatus(t("settings.providers.fetchModelsUnsupported"));
                  return;
                }
                const fetched = fetchedProviderModels(result?.models);
                if (!fetched.length) {
                  setFetchStatus(t("settings.providers.fetchModelsEmpty"));
                  return;
                }
                const currentIds = providerModelIdsFromText(models.value);
                const baseIds = hasProviderDiscoveryPlaceholder(record, currentIds) ? [] : currentIds;
                const nextIds = [...baseIds, ...fetched.map((model) => model.id)]
                  .filter((id, index, all) => all.indexOf(id) === index);
                models.value = nextIds.join("\n");
                for (const model of fetched) fetchedModelMetadata.set(model.id, model);
                setFetchStatus(
                  result?.truncated === true
                    ? fillProviderMessage("settings.providers.fetchModelsTruncated", { count: fetched.length })
                    : fillProviderMessage("settings.providers.fetchModelsSuccess", { count: fetched.length }),
                  "success",
                );
              } catch (error) {
                if (serial !== modelFetchSerial || !form.isConnected) return;
                setFetchStatus(
                  fillProviderMessage("settings.providers.fetchModelsFailed", { error: providerFetchErrorText(error) }),
                  "error",
                );
              } finally {
                if (serial !== modelFetchSerial || !form.isConnected) return;
                fetchingModels = false;
                fetchModels.removeAttribute("aria-busy");
                syncModelFetchAvailability();
              }
            })();
          });
          syncModelFetchAvailability();
        }

        const note = document.createElement("p");
        note.className = "provider-editor-note";
        note.setAttribute("data-i18n", "settings.providers.keyNote");
        note.textContent = t("settings.providers.keyNote");
        form.appendChild(note);

        const actions = document.createElement("div");
        actions.className = "provider-editor-actions";
        const cancel = document.createElement("button");
        cancel.type = "button";
        cancel.className = "btn ghost";
        cancel.setAttribute("data-i18n", "settings.providers.cancel");
        cancel.textContent = t("settings.providers.cancel");
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "btn ghost";
        remove.setAttribute("data-i18n", "settings.providers.remove");
        remove.textContent = t("settings.providers.remove");
        remove.hidden = Boolean(record[PROVIDER_DRAFT]);
        const save = document.createElement("button");
        save.type = "submit";
        save.className = "btn primary";
        save.setAttribute("data-i18n", "settings.providers.save");
        save.textContent = t("settings.providers.save");
        actions.append(cancel, remove, save);
        form.appendChild(actions);

        const syncFormatVisibility = () => {
          requestFormatField.hidden = type.value !== "codex";
        };
        type.addEventListener("change", syncFormatVisibility);
        syncFormatVisibility();

        cancel.addEventListener("click", () => {
          invalidateModelFetch();
          if (record[PROVIDER_DRAFT]) {
            providerState = providerState.filter((item) => item !== record);
            renderProviders();
          } else {
            form.hidden = true;
            card.querySelector('[data-provider-action="edit"]')?.removeAttribute("disabled");
          }
        });
        remove.addEventListener("click", () => {
          void (async () => {
            const nameText = providerString(record, "name", "label") || providerId(record);
            if (!(await appConfirm({
              title: lang === "en" ? "Delete provider" : "删除供应商",
              message: lang === "en" ? `Delete provider “${nameText}”?` : `确定删除供应商“${nameText}”？`,
              confirmLabel: lang === "en" ? "Delete" : "删除",
              cancelLabel: lang === "en" ? "Cancel" : "取消",
              danger: true,
            }))) return;
            invalidateModelFetch();
            remove.disabled = true;
            try {
              await saveProviderState(
                providerState.filter((item) => item !== record),
                [providerId(record)],
              );
              toast(fillProviderMessage("settings.providers.removed", { name: nameText }));
            } catch (error) {
              remove.disabled = false;
              await appError(error, lang === "en" ? "Could not delete provider" : "删除供应商失败");
            }
          })();
        });
        form.addEventListener("submit", (event) => {
          event.preventDefault();
          invalidateModelFetch();
          void saveProviderEditor(record, form, fetchedModelMetadata);
        });
        return form;
      }

      function fillProviderMessage(key, values) {
        return Object.entries(values || {}).reduce((message, [name, value]) => message.replace(`{${name}}`, String(value)), t(key));
      }

      function providerErrorText(error) {
        const raw = error instanceof Error ? error.message : typeof error === "string" ? error : JSON.stringify(error);
        return String(raw || "Unknown error").replace(/\s+/g, " ").slice(0, 280);
      }

      function providerImportPreviewItems(value) {
        if (!Array.isArray(value)) return [];
        const output = [];
        const seen = new Set();
        for (const item of value) {
          if (!providerObject(item)) continue;
          const id = typeof item.id === "string" ? item.id.trim() : "";
          if (!/^[A-Za-z0-9._-]{1,128}$/.test(id) || seen.has(id)) continue;
          seen.add(id);
          const host = typeof item.host === "string" ? item.host.trim() : "";
          const name = typeof item.name === "string" && item.name.trim() ? item.name.trim() : id;
          const protocol = typeof item.protocol === "string" && item.protocol.trim()
            ? item.protocol.trim()
            : "openai-responses";
          const rawApiRoot = typeof item.apiRoot === "string" ? item.apiRoot.trim() : "";
          const apiRoot = PROVIDER_IMPORT_PUBLIC_API_ROOTS.has(rawApiRoot) ? rawApiRoot : "";
          const modelCount = Number.isSafeInteger(item.modelCount) && item.modelCount >= 0
            ? Math.min(item.modelCount, 256)
            : 0;
          output.push({
            id,
            name,
            host,
            apiRoot,
            protocol,
            modelCount,
            conflict: item.conflict === true,
            hasCredential: item.hasCredential === true,
            requiresCredentialReentry: item.requiresCredentialReentry === true,
          });
        }
        return output;
      }

      function providerImportErrorText(error) {
        const raw = providerErrorText(error);
        if (/provider_import_(?:preview|apply)|unknown command|not allowed/i.test(raw)) {
          return t("settings.providers.unavailable");
        }
        return raw;
      }

      function restoreProviderImportFocus() {
        const trigger = providerImportReturnFocus;
        providerImportReturnFocus = null;
        window.requestAnimationFrame(() => {
          const fallback = document.getElementById("btnSyncProviders");
          const target = trigger instanceof HTMLElement && trigger.isConnected ? trigger : fallback;
          if (!(target instanceof HTMLElement) || target.hasAttribute("disabled")) return;
          target.focus({ preventScroll: true });
        });
      }

      function clearProviderImportPreview({ restoreFocus = false } = {}) {
        providerImportToken = "";
        providerImportItems = [];
        providerImportBusy = false;
        if (providerImportPreview) {
          providerImportPreview.hidden = true;
          providerImportPreview.removeAttribute("aria-busy");
          providerImportPreview.replaceChildren();
        }
        if (restoreFocus) restoreProviderImportFocus();
      }

      function renderProviderImportPreview(skipped = 0) {
        if (!providerImportPreview || !providerImportToken || !providerImportItems.length) return;
        providerImportPreview.hidden = false;
        providerImportPreview.replaceChildren();

        const head = document.createElement("div");
        head.className = "provider-import-head";
        const heading = document.createElement("h3");
        heading.textContent = t("settings.providers.importTitle");
        const close = document.createElement("button");
        close.type = "button";
        close.className = "btn ghost";
        close.textContent = t("settings.providers.importCancel");
        close.addEventListener("click", () => {
          if (!providerImportBusy) clearProviderImportPreview({ restoreFocus: true });
        });
        head.append(heading, close);

        const note = document.createElement("p");
        note.className = "provider-import-note";
        note.textContent = t("settings.providers.importNote");
        const status = document.createElement("p");
        status.className = "provider-import-status";
        status.setAttribute("role", "status");
        status.setAttribute("aria-live", "polite");
        if (skipped > 0) {
          status.textContent = fillProviderMessage("settings.providers.importSkipped", { count: skipped });
        }

        const list = document.createElement("div");
        list.className = "provider-import-list";
        list.setAttribute("role", "list");
        const actions = document.createElement("div");
        actions.className = "provider-import-actions";
        const cancel = document.createElement("button");
        cancel.type = "button";
        cancel.className = "btn ghost";
        cancel.textContent = t("settings.providers.importCancel");
        cancel.addEventListener("click", () => {
          if (!providerImportBusy) clearProviderImportPreview({ restoreFocus: true });
        });
        const apply = document.createElement("button");
        apply.type = "button";
        apply.className = "btn primary";
        actions.append(cancel, apply);

        const selectedIds = () => [...list.querySelectorAll("input[data-provider-import-id]")]
          .filter((input) => input.checked)
          .map((input) => input.dataset.providerImportId)
          .filter(Boolean);
        const updateApply = () => {
          const count = selectedIds().length;
          apply.disabled = providerImportBusy || count === 0;
          apply.textContent = fillProviderMessage("settings.providers.importApply", { count });
          for (const input of list.querySelectorAll("input[data-provider-import-id]")) input.disabled = providerImportBusy;
          close.disabled = providerImportBusy;
          cancel.disabled = providerImportBusy;
          providerImportPreview.toggleAttribute("aria-busy", providerImportBusy);
        };

        providerImportItems.forEach((item, index) => {
          const row = document.createElement("article");
          row.className = "provider-import-row";
          row.setAttribute("role", "listitem");
          const checkbox = document.createElement("input");
          checkbox.type = "checkbox";
          checkbox.id = `provider-import-${index}`;
          checkbox.dataset.providerImportId = item.id;
          // Conflicts default to skip, so an existing configuration is never
          // overwritten without a deliberate per-provider choice.
          checkbox.checked = !item.conflict;
          const copy = document.createElement("label");
          copy.className = "provider-import-copy";
          copy.htmlFor = checkbox.id;
          const name = document.createElement("strong");
          name.textContent = item.name;
          const summary = document.createElement("small");
          summary.textContent = `${item.host || item.id} · ${item.protocol} · ${fillProviderMessage("settings.providers.importModels", { count: item.modelCount })}`;
          copy.append(name, summary);
          if (item.apiRoot) {
            const apiRoot = document.createElement("small");
            apiRoot.textContent = fillProviderMessage("settings.providers.importApiRoot", { path: item.apiRoot });
            copy.append(apiRoot);
          }
          const detail = document.createElement("small");
          detail.textContent = item.conflict
            ? t("settings.providers.importConflict")
            : t("settings.providers.importNew");
          copy.append(detail);
          if (item.hasCredential) {
            const credential = document.createElement("small");
            credential.textContent = t("settings.providers.importCredential");
            copy.append(credential);
          }
          if (item.requiresCredentialReentry) {
            const credentialReentry = document.createElement("small");
            credentialReentry.className = "provider-import-reentry";
            credentialReentry.textContent = t("settings.providers.importCredentialReentry");
            copy.append(credentialReentry);
          }
          const action = document.createElement("span");
          action.className = "provider-import-action";
          const syncAction = () => {
            const key = checkbox.checked
              ? (item.conflict ? "settings.providers.importUpdate" : "settings.providers.importAdd")
              : "settings.providers.importSkip";
            action.dataset.action = checkbox.checked ? (item.conflict ? "update" : "add") : "skip";
            action.textContent = t(key);
            updateApply();
          };
          checkbox.addEventListener("change", syncAction);
          row.append(checkbox, copy, action);
          list.appendChild(row);
          syncAction();
        });

        apply.addEventListener("click", () => {
          void (async () => {
            const providerIds = selectedIds();
            if (!providerIds.length) {
              status.textContent = t("settings.providers.importNone");
              status.dataset.state = "error";
              status.setAttribute("role", "alert");
              return;
            }
            if (!(await appConfirm({
              title: t("settings.providers.importTitle") || (lang === "en" ? "Import providers" : "导入供应商"),
              message: fillProviderMessage("settings.providers.importConfirm", { count: providerIds.length }),
              confirmLabel: lang === "en" ? "Import" : "导入",
              cancelLabel: lang === "en" ? "Cancel" : "取消",
              danger: false,
            }))) return;
            providerImportBusy = true;
            delete status.dataset.state;
            status.setAttribute("role", "status");
            status.textContent = "";
            updateApply();
            try {
              const result = await providerInvoke("provider_import_apply", {
                importToken: providerImportToken,
                providerIds,
              });
              const added = Number.isSafeInteger(result?.added) ? result.added : 0;
              const updated = Number.isSafeInteger(result?.updated) ? result.updated : 0;
              clearProviderImportPreview({ restoreFocus: true });
              await loadProviderSettings();
              toast(fillProviderMessage("settings.providers.importApplied", { added, updated }));
            } catch (error) {
              providerImportBusy = false;
              status.textContent = fillProviderMessage("settings.providers.importFailed", { error: providerImportErrorText(error) });
              status.dataset.state = "error";
              status.setAttribute("role", "alert");
              updateApply();
            }
          })();
        });

        providerImportPreview.append(head, note, status, list, actions);
        updateApply();
        const firstCheckbox = list.querySelector("input[data-provider-import-id]");
        window.requestAnimationFrame(() => {
          if (!providerImportBusy && firstCheckbox instanceof HTMLInputElement && firstCheckbox.isConnected) {
            firstCheckbox.focus({ preventScroll: true });
          }
        });
      }

      async function openProviderImport(button) {
        if (providerImportBusy) return;
        providerImportReturnFocus = button;
        providerImportBusy = true;
        button.disabled = true;
        button.setAttribute("aria-busy", "true");
        try {
          const preview = await providerInvoke("provider_import_preview");
          if (!preview) {
            toast(t("settings.providers.importCancelled"));
            restoreProviderImportFocus();
            return;
          }
          const token = typeof preview.importToken === "string" ? preview.importToken.trim() : "";
          const items = providerImportPreviewItems(preview.providers);
          if (!token || !items.length) {
            throw new Error("provider import preview is invalid");
          }
          providerImportToken = token;
          providerImportItems = items;
          providerImportBusy = false;
          renderProviderImportPreview(Number.isSafeInteger(preview.skipped) ? Math.max(0, preview.skipped) : 0);
          providerImportPreview?.scrollIntoView({ block: "nearest", behavior: "smooth" });
        } catch (error) {
          toast(fillProviderMessage("settings.providers.importFailed", { error: providerImportErrorText(error) }));
        } finally {
          providerImportBusy = false;
          button.disabled = false;
          button.removeAttribute("aria-busy");
        }
      }

      function providerPayload(state) {
        return state.map((record) => {
          const copy = { ...record };
          delete copy.__testStatus;
          delete copy.__testDetail;
          delete copy[PROVIDER_DRAFT];
          return copy;
        });
      }

      // Duplicate only copies renderer-visible, non-secret fields into a new
      // unsaved draft. Native-held API keys and custom secret headers never
      // leave the original provider record and are not inherited by the copy.
      function allocateUniqueProviderId(preferredBase) {
        const used = new Set(providerState.map((item) => providerId(item)).filter(Boolean));
        let base = String(preferredBase || "custom-copy").trim();
        base = base.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");
        if (!base || !/^[A-Za-z0-9._-]{1,128}$/.test(base)) base = "custom-copy";
        if (base.length > 110) base = base.slice(0, 110);
        if (!used.has(base)) return base;
        let index = 2;
        while (index < 10_000) {
          const candidate = `${base}-${index}`;
          if (candidate.length <= 128 && !used.has(candidate)) return candidate;
          index += 1;
        }
        return `copy-${Date.now().toString(36)}`.slice(0, 128);
      }

      function stripProviderSecretsForDuplicate(value, inHeaders = false) {
        if (Array.isArray(value)) {
          return value
            .map((item) => stripProviderSecretsForDuplicate(item, inHeaders))
            .filter((item) => item !== undefined);
        }
        if (!value || typeof value !== "object") return value;
        const secretKeys = new Set([
          "apikey",
          "api_key",
          "key",
          "token",
          "accesstoken",
          "access_token",
          "secret",
          "password",
          "value",
        ]);
        const output = {};
        for (const [key, item] of Object.entries(value)) {
          const lowered = key.toLowerCase();
          if (lowered.endsWith("configured") && secretKeys.has(lowered.slice(0, -10))) continue;
          if (secretKeys.has(lowered) && (inHeaders || typeof item === "string" || item === true || item === false)) {
            continue;
          }
          if (["headers", "customheaders", "custom_headers"].includes(lowered)) {
            const cleaned = stripProviderSecretsForDuplicate(item, true);
            if (Array.isArray(cleaned) ? cleaned.length : cleaned && typeof cleaned === "object" && Object.keys(cleaned).length) {
              output[key] = cleaned;
            }
            continue;
          }
          output[key] = stripProviderSecretsForDuplicate(item, inHeaders);
        }
        return output;
      }

      function cloneProviderAsDraft(record) {
        const sourceId = providerId(record) || "custom";
        const sourceName = providerString(record, "name", "label") || sourceId;
        const payload = providerPayload([record])[0] || {};
        const draft = stripProviderSecretsForDuplicate(payload);
        draft.id = allocateUniqueProviderId(`${sourceId}-copy`);
        draft.name = `${sourceName}${t("settings.providers.copySuffix")}`;
        draft.default = false;
        draft.isDefault = false;
        delete draft.__testStatus;
        delete draft.__testDetail;
        delete draft.modelDiscoveryPlaceholder;
        const modelIds = providerModelIds(draft);
        if (!modelIds.length) {
          draft.models = [PROVIDER_DISCOVERY_PLACEHOLDER];
          draft.activeModels = [PROVIDER_DISCOVERY_PLACEHOLDER];
          draft.modelDiscoveryPlaceholder = true;
        }
        draft[PROVIDER_DRAFT] = true;
        return draft;
      }

      function duplicateProvider(record) {
        if (record[PROVIDER_DRAFT]) {
          toast(lang === "en" ? "Save this draft before duplicating it." : "请先保存当前草稿，再复制供应商。");
          return;
        }
        const sourceName = providerString(record, "name", "label") || providerId(record);
        const draft = cloneProviderAsDraft(record);
        providerState = [...providerState, draft];
        renderProviders(draft.id);
        toast(fillProviderMessage("settings.providers.duplicated", { name: sourceName }));
        window.requestAnimationFrame(() => {
          const card = providerList?.querySelector(`[data-provider-id="${CSS.escape(providerId(draft))}"]`);
          card?.querySelector("[data-provider-field=name]")?.focus();
        });
      }

      async function saveProviderState(nextState, removedProviderIds = []) {
        const explicitRemovals = [...new Set(
          (Array.isArray(removedProviderIds) ? removedProviderIds : [])
            .filter((id) => typeof id === "string" && /^[A-Za-z0-9._-]{1,128}$/.test(id)),
        )];
        await providerInvoke("settings_save_providers", {
          payload: providerPayload(nextState),
          removedProviderIds: explicitRemovals,
        });
        await loadProviderSettings();
      }

      function providerConfiguredFlagKeys(record) {
        return Object.keys(record).filter((key) => /^(?:apiKey|api_key|key|token|accessToken|access_token|secret|password)Configured$/i.test(key));
      }

      async function saveProviderEditor(record, form, fetchedModelMetadata = new Map()) {
        const value = (field) => form.querySelector(`[data-provider-field="${field}"]`);
        const name = value("name")?.value.trim() || "";
        const id = value("id")?.value.trim() || "";
        const type = value("type")?.value || "codex";
        const baseUrl = value("baseUrl")?.value.trim() || "";
        const modelIds = providerModelIdsFromText(value("models")?.value);
        let parsedUrl;
        try { parsedUrl = new URL(baseUrl); } catch { parsedUrl = null; }
        if (!name || !/^[A-Za-z0-9._-]{1,128}$/.test(id) || !parsedUrl || !["http:", "https:"].includes(parsedUrl.protocol) || parsedUrl.username || parsedUrl.password || !modelIds.length) {
          toast(t("settings.providers.invalid"));
          return;
        }
        const duplicate = providerState.some((item) => item !== record && providerId(item) === id);
        if (duplicate) {
          toast(lang === "en" ? "Provider ID already exists." : "供应商标识已存在。");
          return;
        }
        const next = { ...record, id, name, type, baseUrl, models: modelIds.map((modelId) => {
          const old = (Array.isArray(record.models) ? record.models : []).find((item) => (typeof item === "string" ? item : providerString(item || {}, "id", "modelId", "model_id")) === modelId);
          const fetched = fetchedModelMetadata.get(modelId);
          if ((old && typeof old === "object") || fetched) {
            return { ...(old && typeof old === "object" ? old : {}), ...(fetched || {}) };
          }
          return modelId;
        }), activeModels: [modelIds[0]], defaultModel: modelIds[0], useSystemProxy: Boolean(value("systemProxy")?.checked) };
        if (type === "codex") {
          next.requestFormat = value("requestFormat")?.value || "openai-responses";
          next.protocol = next.requestFormat;
        } else {
          delete next.requestFormat;
          delete next.request_format;
          next.protocol = type === "gemini" ? "google-generative-ai" : "anthropic-messages";
        }
        const enteredKey = value("apiKey")?.value.trim() || "";
        if (enteredKey) {
          next.apiKey = enteredKey;
          for (const key of providerConfiguredFlagKeys(next)) delete next[key];
        } else if (record[PROVIDER_DRAFT]) {
          delete next.apiKey;
        }
        if (Boolean(value("default")?.checked)) {
          for (const item of providerState) {
            if (item !== record) {
              item.default = false;
              item.isDefault = false;
            }
          }
          next.default = true;
          next.isDefault = true;
        } else {
          next.default = false;
          next.isDefault = false;
        }
        if (!hasProviderDiscoveryPlaceholder(record, modelIds)) {
          delete next.modelDiscoveryPlaceholder;
        }
        delete next[PROVIDER_DRAFT];
        const index = providerState.indexOf(record);
        if (index < 0) return;
        providerState = providerState.slice();
        providerState[index] = next;
        const saveButton = form.querySelector("button[type=submit]");
        if (saveButton) saveButton.disabled = true;
        try {
          await saveProviderState(providerState);
          toast(fillProviderMessage("settings.providers.saved", { name }));
        } catch (error) {
          // Keep the editor open and restore the pre-save object on failure.
          providerState[index] = record;
          toast(providerErrorText(error));
          if (saveButton) saveButton.disabled = false;
        }
      }

      function renderProviders(openId) {
        if (!providerList) return;
        providerList.replaceChildren();
        if (!providerState.length) {
          const empty = document.createElement("div");
          empty.className = "provider-empty";
          empty.setAttribute("data-i18n", "settings.providers.empty");
          empty.textContent = t("settings.providers.empty");
          providerList.appendChild(empty);
          return;
        }
        for (const record of providerState) {
          const card = document.createElement("article");
          card.className = "provider-card";
          card.dataset.providerId = providerId(record);
          const head = document.createElement("div");
          head.className = "provider-card-head";
          const logo = document.createElement("div");
          logo.className = "provider-logo";
          logo.textContent = providerInitials(record);
          const copy = document.createElement("div");
          copy.className = "provider-copy";
          const strong = document.createElement("strong");
          strong.textContent = providerString(record, "name", "label") || providerId(record);
          const small = document.createElement("small");
          small.textContent = `${providerDisplayUrl(record)} · ${providerProtocolLabel(record)}`;
          copy.append(strong, small);
          const status = providerStatus(record);
          const statusEl = document.createElement("span");
          statusEl.className = `pill ${status.className}`.trim();
          statusEl.dataset.providerStatus = "true";
          statusEl.setAttribute("data-i18n", status.key);
          statusEl.textContent = t(status.key);
          head.append(logo, copy, statusEl);
          card.appendChild(head);

          const meta = document.createElement("div");
          meta.className = "provider-meta";
          if (record.default === true || record.isDefault === true) {
            const defaultTag = providerTag(t("settings.providers.defaultBadge"));
            defaultTag.setAttribute("data-i18n", "settings.providers.defaultBadge");
            meta.appendChild(defaultTag);
          }
          const models = providerModelIds(record);
          if (models[0]) meta.appendChild(providerTag(models[0], true));
          const count = providerTag(`${models.length || 0} ${t("settings.providers.models")}`);
          count.replaceChildren(document.createTextNode(`${models.length || 0} `));
          const modelsLabel = document.createElement("span");
          modelsLabel.setAttribute("data-i18n", "settings.providers.models");
          modelsLabel.textContent = t("settings.providers.models");
          count.appendChild(modelsLabel);
          meta.appendChild(count);
          card.appendChild(meta);

          const actions = document.createElement("div");
          actions.className = "provider-actions";
          const test = document.createElement("button");
          test.type = "button";
          test.className = "btn";
          test.dataset.providerAction = "test";
          test.setAttribute("data-i18n", "settings.providers.test");
          test.textContent = t("settings.providers.test");
          test.disabled = Boolean(record[PROVIDER_DRAFT]);
          test.addEventListener("click", () => void testProvider(record, card, test));
          const edit = document.createElement("button");
          edit.type = "button";
          edit.className = "btn";
          edit.dataset.providerAction = "edit";
          edit.setAttribute("data-i18n", "settings.providers.edit");
          edit.textContent = t("settings.providers.edit");
          edit.addEventListener("click", () => {
            openProviderEditorView(record, edit);
          });
          const duplicate = document.createElement("button");
          duplicate.type = "button";
          duplicate.className = "btn";
          duplicate.dataset.providerAction = "duplicate";
          duplicate.setAttribute("data-i18n", "settings.providers.duplicate");
          duplicate.textContent = t("settings.providers.duplicate");
          duplicate.disabled = Boolean(record[PROVIDER_DRAFT]);
          duplicate.title = t("settings.providers.duplicate");
          duplicate.addEventListener("click", () => duplicateProvider(record));
          actions.append(test, edit, duplicate);
          card.appendChild(actions);
          providerList.appendChild(card);
        }
      }

      async function loadProviderSettings(options = {}) {
        const serial = ++providerLoadSerial;
        if (providerList && !providerState.length) {
          providerList.replaceChildren();
          const loading = document.createElement("div");
          loading.className = "provider-empty";
          loading.setAttribute("data-i18n", "settings.providers.loading");
          loading.textContent = t("settings.providers.loading");
          providerList.appendChild(loading);
        }
        try {
          const settings = await providerInvoke("settings_load_all");
          if (serial !== providerLoadSerial) return;
          providerState = providerRecords(settings?.providers);
          renderProviders(options.openId);
          window.dispatchEvent(new CustomEvent("novavei:providers-changed"));
        } catch (error) {
          if (serial !== providerLoadSerial) return;
          providerState = [];
          providerList?.replaceChildren();
          const failure = document.createElement("div");
          failure.className = "provider-empty";
          failure.textContent = fillProviderMessage("settings.providers.loadFailed", { error: providerErrorText(error) });
          providerList?.appendChild(failure);
        }
      }

      async function testProvider(record, card, button) {
        const name = providerString(record, "name", "label") || providerId(record);
        if (!providerMayBeKeyless(record) && !providerConfigured(record)) {
          const message = fillProviderMessage("settings.providers.notConfigured", { name });
          card.querySelector("[data-provider-status]")?.setAttribute("title", message);
          toast(message);
          record.__testStatus = "failed";
          renderProviders();
          return;
        }
        button.disabled = true;
        record.__testStatus = "testing";
        renderProviders();
        try {
          const result = await providerInvoke("provider_test", { providerId: providerId(record) });
          if (result && result.ok === false) throw new Error(result.error || result.message || "provider test failed");
          const detail = [
            result?.status ? `HTTP ${result.status}` : "OK",
            Number.isFinite(Number(result?.latencyMs)) ? `${Math.round(Number(result.latencyMs))} ms` : "",
            Number.isFinite(Number(result?.modelCount)) ? `${result.modelCount} ${t("settings.providers.models")}` : "",
          ].filter(Boolean).join(" · ");
          record.__testStatus = "ok";
          toast(fillProviderMessage("settings.providers.testOk", { name, detail }));
        } catch (error) {
          const rawMessage = providerErrorText(error);
          const message = /provider_test|unknown command|not allowed/i.test(rawMessage)
            ? t("settings.providers.testUnavailable")
            : rawMessage;
          record.__testStatus = "failed";
          toast(fillProviderMessage("settings.providers.testFailed", { name, error: message }));
        } finally {
          renderProviders();
        }
      }

      document.getElementById("btnAddProvider")?.addEventListener("click", () => {
        const used = new Set(providerState.map((item) => providerId(item)));
        let n = providerState.length + 1;
        while (used.has(`custom-${n}`)) n += 1;
        const draft = {
          id: `custom-${n}`,
          name: `${t("settings.providers.newName")} ${n}`,
          type: "codex",
          protocol: "openai-responses",
          requestFormat: "openai-responses",
          baseUrl: "https://api.example.com/v1",
          models: [],
          activeModels: [],
          promptCachingEnabled: true,
          enabled: true,
          [PROVIDER_DRAFT]: true,
        };
        providerState = [...providerState, draft];
        renderProviders();
        openProviderEditorView(draft, document.getElementById("btnAddProvider"));
      });

      document.getElementById("btnRefreshProviders")?.addEventListener("click", () => {
        void loadProviderSettings();
      });
      document.getElementById("btnSyncProviders")?.addEventListener("click", (event) => {
        const button = event.currentTarget;
        if (button instanceof HTMLButtonElement) void openProviderImport(button);
      });

      // Hydrate once in the desktop shell. A normal browser gets an explicit
      // unavailable/error state instead of fabricated provider cards.
      void loadProviderSettings();

      // Search palette (Ctrl/Cmd+K)
      const searchPalette = document.getElementById("searchPalette");
      const sessionSearch = document.getElementById("sessionSearch");
      function openSearchPalette() {
        if (!searchPalette || !sessionSearch) return;
        searchPalette.hidden = false;
        searchPalette.classList.add("show");
        if (window.matchMedia("(max-width: 820px)").matches) {
          workbench.classList.add("side-open");
          syncSideToggle();
        }
        requestAnimationFrame(() => {
          sessionSearch.focus();
          sessionSearch.select?.();
        });
      }
      function closeSearchPalette() {
        if (!searchPalette) return;
        searchPalette.classList.remove("show");
        searchPalette.hidden = true;
      }
      searchPalette?.addEventListener("click", (ev) => {
        if (ev.target === searchPalette) closeSearchPalette();
      });
      document.getElementById("btnOpenSearch")?.addEventListener("click", () => {
        openSearchPalette();
      });

      // Escape closes overlay / expands focus to chat
      document.addEventListener("keydown", (e) => {
        if (e.key === "Escape") {
          if (modelPopover.classList.contains("show")) {
            closeModelPopover(true);
          } else if (searchPalette?.classList.contains("show")) {
            closeSearchPalette();
          } else if (typeof closeProviderEditorView === "function" && document.getElementById("providerEditorModal")?.classList.contains("show")) {
            // Prefer dismissing the provider editor shell before the whole Settings overlay.
            e.preventDefault();
            closeProviderEditorView();
          } else if (Object.values(overlays).some((o) => o.classList.contains("show"))) {
            closeOverlays();
            toast("已关闭浮层");
          } else if (workbench.classList.contains("side-open")) {
            workbench.classList.remove("side-open");
            syncSideToggle();
            document.getElementById("btnToggleSide").focus();
          }
        }
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
          e.preventDefault();
          if (searchPalette?.classList.contains("show")) closeSearchPalette();
          else openSearchPalette();
        }
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
          e.preventDefault();
          document.getElementById("btnNewChat").click();
        }
        if ((e.ctrlKey || e.metaKey) && e.key === ",") {
          e.preventDefault();
          openOverlay("settings");
        }
        if (
          (e.ctrlKey || e.metaKey) &&
          !e.altKey &&
          !e.shiftKey &&
          !e.isComposing &&
          !document.querySelector("dialog[open]") &&
          e.key.toLowerCase() === "b"
        ) {
          e.preventDefault();
          toggleConversationOnlyMode();
        }
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "i") {
          e.preventDefault();
          toggleDock();
        }
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "l") {
          e.preventDefault();
          setTheme(document.documentElement.dataset.theme === "light" ? "dark" : "light");
        }
      });

      // ── Floor navigation (user-turn rail, NovaVei-style) ──
      (function initFloorNav() {
        const PREVIEW_MAX = 24;
        const MIN_MARKERS = 8;
        const MAX_MARKERS = 40;
        const MAX_MARKERS_TOUCH = 12;
        const MARKER_SLOT_PX = 9.5;
        const COLLAPSE_DELAY_MS = 160;
        const TOUCH_REVEAL_MS = 1400;
        const STORAGE_KEY = "novavei.floor-bookmarks.v1";
        const MAX_CONVERSATIONS = 200;
        const PIN_SVG =
          '<svg class="ico" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 17v5M9 10.76V7a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v3.76a2 2 0 0 0 .5 1.32l1.7 1.87A1 1 0 0 1 16.5 16h-9a1 1 0 0 1-.7-1.71l1.7-1.87A2 2 0 0 0 9 10.76z"/></svg>';

        const transcript = document.getElementById("transcript");
        const axis = document.getElementById("transcriptAxis") || document.querySelector(".axis");
        const nav = document.getElementById("floorNav");
        if (!transcript || !axis || !nav) return;

        const isCoarse =
          typeof window.matchMedia === "function" &&
          window.matchMedia("(hover: none), (pointer: coarse)").matches;
        if (isCoarse) nav.classList.add("is-coarse");

        let expanded = false;
        let activeRowKey = null;
        let collapseTimer = null;
        let revealTimer = null;
        let touchRevealed = !isCoarse;
        let markerBudget = isCoarse ? MAX_MARKERS_TOUCH : MAX_MARKERS;
        let floorsCache = [];

        function conversationId() {
          const nativeHost = window.__novaveiHost;
          if (nativeHost) return nativeHost.getSessionId?.()?.trim() || "native-current";
          return (
            document.getElementById("chatTitle")?.textContent?.trim() ||
            document.querySelector(".session[aria-current='page'] strong")?.textContent?.trim() ||
            "current"
          );
        }

        function buildFloorPreview(text) {
          const collapsed = String(text || "").replace(/\s+/g, " ").trim();
          if (!collapsed) return "…";
          const chars = Array.from(collapsed);
          return chars.length > PREVIEW_MAX
            ? chars.slice(0, PREVIEW_MAX).join("") + "…"
            : collapsed;
        }

        function ensureFloorIds() {
          axis.querySelectorAll(".msg-user").forEach((el, index) => {
            if (!el.dataset.floorId) {
              el.dataset.floorId =
                el.dataset.messageId ||
                "floor-auto-" + index + "-" + Math.random().toString(36).slice(2, 7);
            }
            if (!el.dataset.messageId) el.dataset.messageId = el.dataset.floorId;
          });
        }

        function collectFloors() {
          ensureFloorIds();
          return [...axis.querySelectorAll(".msg-user")].map((el) => ({
            rowKey: el.dataset.floorId,
            messageId: el.dataset.messageId || el.dataset.floorId,
            preview: buildFloorPreview(el.textContent || ""),
            el,
          }));
        }

        function sampleFloorEntries(floors, maxMarkers, mustKeep) {
          if (maxMarkers <= 0) return [];
          if (floors.length <= maxMarkers) return floors;
          const picked = new Set();
          const lastIndex = floors.length - 1;
          for (let i = 0; i < maxMarkers; i++) {
            picked.add(Math.round((i * lastIndex) / (maxMarkers - 1 || 1)));
          }
          return floors.filter(
            (floor, index) => picked.has(index) || mustKeep.has(floor.rowKey),
          );
        }

        function resolveNearestSampledRowKey(floors, sampled, activeKey) {
          if (!activeKey || !sampled.length) return null;
          if (sampled.some((f) => f.rowKey === activeKey)) return activeKey;
          const activeIndex = floors.findIndex((f) => f.rowKey === activeKey);
          if (activeIndex === -1) return null;
          let nearest = null;
          let nearestDistance = Number.POSITIVE_INFINITY;
          for (const marker of sampled) {
            const markerIndex = floors.findIndex((f) => f.rowKey === marker.rowKey);
            if (markerIndex === -1) continue;
            const distance = Math.abs(markerIndex - activeIndex);
            if (distance < nearestDistance) {
              nearestDistance = distance;
              nearest = marker.rowKey;
            }
          }
          return nearest;
        }

        function readBookmarksMap() {
          try {
            const raw = localStorage.getItem(STORAGE_KEY);
            if (!raw) return {};
            const parsed = JSON.parse(raw);
            const conversations = parsed && parsed.conversations;
            if (!conversations || typeof conversations !== "object") return {};
            const result = {};
            for (const [id, ids] of Object.entries(conversations)) {
              if (!Array.isArray(ids)) continue;
              const clean = ids.filter((x) => typeof x === "string" && x.length > 0);
              if (clean.length) result[id] = clean;
            }
            return result;
          } catch {
            return {};
          }
        }

        function getBookmarks() {
          return new Set(readBookmarksMap()[conversationId()] || []);
        }

        function toggleBookmark(messageId) {
          if (!messageId) return;
          const map = readBookmarksMap();
          const cid = conversationId();
          const next = new Set(map[cid] || []);
          if (next.has(messageId)) next.delete(messageId);
          else next.add(messageId);
          if (next.size === 0) delete map[cid];
          else map[cid] = [...next];
          // LRU-ish: keep insertion order of keys; re-insert current conversation last
          const ordered = {};
          for (const [id, ids] of Object.entries(map)) {
            if (id !== cid) ordered[id] = ids;
          }
          if (map[cid]) ordered[cid] = map[cid];
          let keys = Object.keys(ordered);
          while (keys.length > MAX_CONVERSATIONS) {
            delete ordered[keys[0]];
            keys = Object.keys(ordered);
          }
          try {
            localStorage.setItem(
              STORAGE_KEY,
              JSON.stringify({ version: 1, conversations: ordered }),
            );
          } catch {
            /* private mode */
          }
          render();
        }

        function cancelCollapse() {
          if (collapseTimer != null) {
            clearTimeout(collapseTimer);
            collapseTimer = null;
          }
        }

        function setExpanded(next) {
          expanded = next;
          if (isCoarse) {
            if (expanded) {
              touchRevealed = true;
              if (revealTimer != null) {
                clearTimeout(revealTimer);
                revealTimer = null;
              }
            } else {
              scheduleTouchHide();
            }
          }
          render();
        }

        function scheduleTouchHide() {
          if (!isCoarse) return;
          if (revealTimer != null) clearTimeout(revealTimer);
          revealTimer = setTimeout(() => {
            revealTimer = null;
            if (!expanded) {
              touchRevealed = false;
              syncVisibility();
            }
          }, TOUCH_REVEAL_MS);
        }

        function syncVisibility() {
          const visible = !isCoarse || touchRevealed;
          nav.classList.toggle("is-hidden", !visible);
          nav.setAttribute("aria-hidden", visible ? "false" : "true");
        }

        function jumpTo(rowKey) {
          const floor = floorsCache.find((f) => f.rowKey === rowKey);
          if (!floor?.el) return;
          floor.el.scrollIntoView({ block: "start", behavior: "smooth" });
          activeRowKey = rowKey;
          if (isCoarse) setExpanded(false);
          else render();
          // re-sync after smooth scroll settles
          setTimeout(reportAnchor, 320);
        }

        function updateMarkerBudget() {
          const max = isCoarse ? MAX_MARKERS_TOUCH : MAX_MARKERS;
          const h = nav.clientHeight || 0;
          const budget = Math.floor((h - 24) / MARKER_SLOT_PX);
          markerBudget = Math.max(MIN_MARKERS, Math.min(max, budget || max));
        }

        function reportAnchor() {
          const floors = floorsCache;
          if (!floors.length) {
            activeRowKey = null;
            return;
          }
          const scrollTop = transcript.scrollTop;
          const viewportHeight = transcript.clientHeight;
          const nearBottom =
            scrollTop + viewportHeight >= transcript.scrollHeight - 32;
          let next = null;
          if (nearBottom) {
            next = floors[floors.length - 1].rowKey;
          } else {
            const line = scrollTop + 8;
            const trRect = transcript.getBoundingClientRect();
            for (const floor of floors) {
              const elRect = floor.el.getBoundingClientRect();
              const topInScroll = elRect.top - trRect.top + scrollTop;
              if (topInScroll <= line) next = floor.rowKey;
              else break;
            }
            if (!next) next = floors[0].rowKey;
          }
          if (next !== activeRowKey) {
            activeRowKey = next;
            // light update: only toggle active classes if DOM already rendered
            const markers = nav.querySelectorAll("[data-floor-key]");
            if (markers.length) {
              const bookmarks = getBookmarks();
              const mustKeep = new Set(
                floors.filter((f) => bookmarks.has(f.messageId)).map((f) => f.rowKey),
              );
              const sampled = sampleFloorEntries(floors, markerBudget, mustKeep);
              const activeMarker = resolveNearestSampledRowKey(
                floors,
                sampled,
                activeRowKey,
              );
              markers.forEach((btn) => {
                const key = btn.getAttribute("data-floor-key");
                btn.classList.toggle("is-active", key === activeMarker || key === activeRowKey);
              });
              nav.querySelectorAll(".floor-nav-row").forEach((row) => {
                row.classList.toggle(
                  "is-active",
                  row.getAttribute("data-floor-key") === activeRowKey,
                );
              });
            } else {
              render();
            }
          }
        }

        function renderPanelRow(floor, bookmarks, isPinnedCopy) {
          const isActive = floor.rowKey === activeRowKey;
          const isBookmarked = bookmarks.has(floor.messageId);
          const row = document.createElement("div");
          row.className = "floor-nav-row" + (isActive ? " is-active" : "");
          row.dataset.floorKey = floor.rowKey;
          if (isActive && !isPinnedCopy) row.dataset.floorActive = "true";

          const jump = document.createElement("button");
          jump.type = "button";
          jump.className = "floor-nav-jump";
          jump.title = floor.preview;
          jump.textContent = floor.preview;
          jump.addEventListener("click", () => jumpTo(floor.rowKey));

          const pin = document.createElement("button");
          pin.type = "button";
          pin.className = "floor-nav-pin" + (isBookmarked ? " is-on" : "");
          pin.setAttribute(
            "aria-label",
            isBookmarked ? t("floor.unpin") : t("floor.pin"),
          );
          pin.title = isBookmarked ? t("floor.unpin") : t("floor.pin");
          pin.innerHTML = PIN_SVG;
          pin.addEventListener("click", (ev) => {
            ev.stopPropagation();
            toggleBookmark(floor.messageId);
          });

          row.appendChild(jump);
          row.appendChild(pin);
          return row;
        }

        function render() {
          floorsCache = collectFloors();
          const floors = floorsCache;
          nav.setAttribute("aria-label", t("floor.navAria"));

          if (floors.length < 2) {
            nav.hidden = true;
            nav.innerHTML = "";
            return;
          }
          nav.hidden = false;
          updateMarkerBudget();
          syncVisibility();

          const bookmarks = getBookmarks();
          const bookmarkedFloors = floors.filter((f) => bookmarks.has(f.messageId));
          const mustKeep = new Set(bookmarkedFloors.map((f) => f.rowKey));
          const collapsedMarkers = sampleFloorEntries(floors, markerBudget, mustKeep);
          const activeMarkerKey = resolveNearestSampledRowKey(
            floors,
            collapsedMarkers,
            activeRowKey,
          );

          nav.innerHTML = "";
          if (expanded) {
            const panel = document.createElement("div");
            panel.className = "floor-nav-panel";
            const scroll = document.createElement("div");
            scroll.className = "floor-nav-panel-scroll";

            if (bookmarkedFloors.length) {
              const pinned = document.createElement("div");
              pinned.className = "floor-nav-pinned";
              const title = document.createElement("div");
              title.className = "floor-nav-pinned-title";
              title.innerHTML = PIN_SVG + "<span></span>";
              title.querySelector("span").textContent = t("floor.pinned");
              pinned.appendChild(title);
              bookmarkedFloors.forEach((floor) => {
                pinned.appendChild(renderPanelRow(floor, bookmarks, true));
              });
              scroll.appendChild(pinned);
            }
            floors.forEach((floor) => {
              scroll.appendChild(renderPanelRow(floor, bookmarks, false));
            });
            panel.appendChild(scroll);
            nav.appendChild(panel);

            const activeEl = scroll.querySelector('[data-floor-active="true"]');
            if (activeEl) {
              requestAnimationFrame(() => {
                activeEl.scrollIntoView({ block: "center", behavior: "auto" });
              });
            }
          } else {
            const rail = document.createElement("div");
            rail.className = "floor-nav-collapsed";
            collapsedMarkers.forEach((floor) => {
              const btn = document.createElement("button");
              btn.type = "button";
              btn.className =
                "floor-nav-marker" +
                (floor.rowKey === activeMarkerKey ? " is-active" : "") +
                (bookmarks.has(floor.messageId) ? " is-pinned" : "");
              btn.dataset.floorKey = floor.rowKey;
              btn.setAttribute("aria-label", floor.preview);
              btn.title = floor.preview;
              btn.addEventListener("click", () => jumpTo(floor.rowKey));
              rail.appendChild(btn);
            });
            if (isCoarse) {
              rail.addEventListener(
                "touchend",
                (ev) => {
                  ev.preventDefault();
                  setExpanded(true);
                },
                { passive: false },
              );
            }
            nav.appendChild(rail);
          }
        }

        nav.addEventListener("mouseenter", () => {
          if (isCoarse) return;
          cancelCollapse();
          setExpanded(true);
        });
        nav.addEventListener("mouseleave", () => {
          if (isCoarse) return;
          cancelCollapse();
          collapseTimer = setTimeout(() => {
            collapseTimer = null;
            setExpanded(false);
          }, COLLAPSE_DELAY_MS);
        });

        document.addEventListener(
          "pointerdown",
          (event) => {
            if (!expanded) return;
            if (event.target instanceof Node && nav.contains(event.target)) return;
            cancelCollapse();
            setExpanded(false);
          },
          true,
        );

        transcript.addEventListener(
          "scroll",
          () => {
            if (isCoarse) {
              touchRevealed = true;
              syncVisibility();
              scheduleTouchHide();
            }
            reportAnchor();
          },
          { passive: true },
        );

        if (typeof ResizeObserver !== "undefined") {
          const ro = new ResizeObserver(() => {
            updateMarkerBudget();
            render();
          });
          ro.observe(nav);
        }

        const mo = new MutationObserver(() => {
          render();
          reportAnchor();
        });
        mo.observe(axis, { childList: true, subtree: false });

        const prevApplyI18n = applyI18n;
        applyI18n = function () {
          prevApplyI18n();
          render();
        };

        render();
        reportAnchor();
        // Re-check after the first layout pass.
        requestAnimationFrame(() => {
          reportAnchor();
          render();
        });

        window.__novaveiFloorNav = {
          refresh() {
            render();
            reportAnchor();
          },
        };
      })();
    })();

// The visual shell owns the static DOM wiring. Loading the typed runtime only
// after that synchronous setup preserves the original browser-preview order
// while allowing Vite to bundle and minify both entries.
void import("./runtime/main.ts");
