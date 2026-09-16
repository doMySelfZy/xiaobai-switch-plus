import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  App as AntdApp,
  Button,
  Dropdown,
  Empty,
  Input,
  Modal,
  Select,
  Space,
  Spin,
  Tabs,
  Tag,
  Typography,
  theme,
} from "antd";
import { Download, FolderOpen, Layers, RefreshCw, Sparkles, Store } from "lucide-react";
import { useTranslation } from "react-i18next";
import { MarketplaceCard } from "@/components/skills/MarketplaceCard";
import { SkillCard } from "@/components/skills/SkillCard";
import {
  SKILL_TARGETS,
  SkillTargetIcon,
  SkillTargetLabel,
  skillTargetLabelKey,
} from "@/components/skills/SkillTargetIcon";
import { invoke, isAppError } from "@/lib/invoke";
import { openExternalUrl } from "@/lib/openUrl";
import { useSkillStore, type SkillMarketplaceSource } from "@/stores/skillStore";
import type { MarketplaceSkill, Skill, SkillTarget } from "@/types/domain";

type TargetFilter = "all" | SkillTarget;

function errorMessage(error: unknown): string {
  if (isAppError(error)) return error.message;
  return error instanceof Error ? error.message : String(error);
}

function githubUrl(repo: string): string {
  return /^https?:\/\//i.test(repo) ? repo : `https://github.com/${repo}`;
}

interface InstallFromUrlBarProps {
  /** 有安装任务在跑时禁用整条输入区。 */
  busy: boolean;
  /** 当前正在安装的 key：URL 就是 source，市场条目是 `repo:target`。 */
  installing: string | null;
  targetMenuItems: { key: string; label: ReactNode }[];
  onInstall: (source: string, target: SkillTarget) => Promise<boolean>;
  onRefresh: () => void;
}

/**
 * 「从 URL 安装」输入条：输入值留在组件内部，击键只重渲染这一行，
 * 不会带着已安装技能列表一起重渲染。
 */
function InstallFromUrlBar({
  busy,
  installing,
  targetMenuItems,
  onInstall,
  onRefresh,
}: InstallFromUrlBarProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState("");
  const trimmed = url.trim();
  const disabled = busy || trimmed.length === 0;

  const handleInstall = async (target: SkillTarget) => {
    const source = url.trim();
    if (!source) return;
    const ok = await onInstall(source, target);
    // 成功后清空输入，但只在用户没有继续编辑时清（与旧行为一致）。
    if (ok) setUrl((current) => (current.trim() === source ? "" : current));
  };

  return (
    <Space.Compact className="mb-2 w-full">
      <Input
        allowClear
        disabled={busy}
        value={url}
        placeholder={t("skills.installUrlPlaceholder")}
        onChange={(event) => setUrl(event.target.value)}
      />
      <Dropdown
        trigger={["click"]}
        disabled={disabled}
        menu={{
          items: targetMenuItems,
          onClick: ({ key }) => void handleInstall(key as SkillTarget),
        }}
      >
        <Button
          type="primary"
          disabled={disabled}
          loading={installing === trimmed}
          icon={<Download size={14} />}
        >
          {t("skills.installFromUrl")}
        </Button>
      </Dropdown>
      <Button
        aria-label={t("skills.refresh")}
        icon={<RefreshCw size={14} />}
        onClick={onRefresh}
      />
    </Space.Compact>
  );
}

export function SkillsPage() {
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const { message, modal } = AntdApp.useApp();
  // 按字段订阅：市场搜索/安装只更新对应字段，不再整页重渲染。
  const skills = useSkillStore((s) => s.skills);
  const loading = useSkillStore((s) => s.loading);
  const marketplaceSkills = useSkillStore((s) => s.marketplaceSkills);
  const marketplaceLoading = useSkillStore((s) => s.marketplaceLoading);
  const selectedSkill = useSkillStore((s) => s.selectedSkill);
  const loadSkills = useSkillStore((s) => s.loadSkills);
  const getSkill = useSkillStore((s) => s.getSkill);
  const clearSelectedSkill = useSkillStore((s) => s.clearSelectedSkill);
  const setSkillEnabled = useSkillStore((s) => s.setSkillEnabled);
  const installSkill = useSkillStore((s) => s.installSkill);
  const uninstallSkill = useSkillStore((s) => s.uninstallSkill);
  const searchMarketplaceAction = useSkillStore((s) => s.searchMarketplace);
  const [pageTab, setPageTab] = useState<"mine" | "market">("mine");
  const [targetFilter, setTargetFilter] = useState<TargetFilter>("all");
  const [busySkill, setBusySkill] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [marketSource, setMarketSource] = useState<SkillMarketplaceSource>("skills.sh");
  const [marketQuery, setMarketQuery] = useState("");
  const marketLoaded = useRef(false);

  useEffect(() => {
    void loadSkills().catch((error) => message.error(errorMessage(error)));
  }, [message, loadSkills]);

  const filteredSkills = useMemo(
    () => targetFilter === "all"
      ? skills
      : skills.filter((skill) => skill.target === targetFilter),
    [skills, targetFilter],
  );

  const targetPath = targetFilter === "all"
    ? null
    : skills.find((skill) => skill.target === targetFilter)?.skillsPath ?? null;

  async function refreshSkills() {
    try {
      await loadSkills(true);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function showDetail(skill: Skill) {
    try {
      await getSkill(skill.target, skill.sourcePath);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function openPath(path: string) {
    try {
      await invoke("open_path", { path });
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function openRepository(repo: string) {
    try {
      await openExternalUrl(githubUrl(repo));
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  async function toggleSkill(skill: Skill, enabled: boolean) {
    setBusySkill(`${skill.target}:${skill.sourcePath}`);
    try {
      await setSkillEnabled(skill.target, skill.sourcePath, enabled);
      message.success(t(enabled ? "skills.enabledSuccess" : "skills.disabledSuccess", { name: skill.name }));
    } catch (error) {
      message.error(errorMessage(error));
    } finally {
      setBusySkill(null);
    }
  }

  function confirmUninstall(skill: Skill) {
    modal.confirm({
      centered: true,
      title: t("skills.uninstallConfirm", { name: skill.name }),
      okText: t("skills.uninstall"),
      okButtonProps: { danger: true },
      cancelText: t("common.cancel"),
      onOk: async () => {
        try {
          await uninstallSkill(skill.target, skill.sourcePath);
          message.success(t("skills.uninstallSuccess", { name: skill.name }));
        } catch (error) {
          message.error(errorMessage(error));
          throw error;
        }
      },
    });
  }

  /** 返回是否成功，供 URL 输入条决定要不要清空输入。 */
  async function install(
    source: string,
    target: SkillTarget,
    key: string,
    skillName?: string,
  ): Promise<boolean> {
    setInstalling(key);
    try {
      const name = await installSkill(source, target, skillName);
      message.success(t("skills.installSuccess", { name }));
      return true;
    } catch (error) {
      message.error(errorMessage(error));
      return false;
    } finally {
      setInstalling(null);
    }
  }

  async function searchMarketplace(query: string, source = marketSource) {
    marketLoaded.current = true;
    setMarketQuery(query);
    try {
      await searchMarketplaceAction(query, source);
    } catch (error) {
      message.error(errorMessage(error));
    }
  }

  function changeMarketSource(source: SkillMarketplaceSource) {
    setMarketSource(source);
    if (marketLoaded.current) void searchMarketplace(marketQuery, source);
  }

  function installMarketplace(skill: MarketplaceSkill, target: SkillTarget) {
    const skillName = marketSource === "skills.sh" ? skill.name : undefined;
    void install(skill.repo, target, `${skill.repo}:${target}`, skillName);
  }

  const targetMenuItems = SKILL_TARGETS.map((target) => ({
    key: target,
    label: <SkillTargetLabel target={target} />,
  }));
  const targetTabItems = [
    {
      key: "all",
      label: (
        <span className="inline-flex items-center gap-2 whitespace-nowrap">
          <Layers size={14} />
          {t("skills.target.all")}
        </span>
      ),
    },
    ...SKILL_TARGETS.map((target) => ({
      key: target,
      label: (
        <span className="inline-flex items-center gap-2 whitespace-nowrap">
          <SkillTargetIcon target={target} />
          {t(skillTargetLabelKey(target))}
        </span>
      ),
    })),
  ];
  const installBusy = installing !== null;

  const mySkills = (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="shrink-0 px-1">
        <InstallFromUrlBar
          busy={installBusy}
          installing={installing}
          targetMenuItems={targetMenuItems}
          onInstall={(source, target) => install(source, target, source)}
          onRefresh={() => void refreshSkills()}
        />
        <Tabs
          className="skill-target-tabs"
          tabBarGutter={32}
          tabBarStyle={{ marginBottom: 8 }}
          activeKey={targetFilter}
          onChange={(key) => setTargetFilter(key as TargetFilter)}
          items={targetTabItems}
        />
      </div>
      <div
        data-testid="skill-list"
        className="min-h-0 flex-1 px-1"
        style={{ flex: "1 1 0%", minHeight: 0, overflowY: "auto" }}
      >
        {targetPath && (
          <div className="mb-2 flex items-center gap-2 rounded-md px-2 py-1" style={{ background: token.colorBgContainer }}>
            <FolderOpen size={14} className="shrink-0" color={token.colorTextSecondary} />
            <Typography.Text type="secondary" ellipsis className="min-w-0 flex-1 text-xs">{targetPath}</Typography.Text>
            <Button type="text" size="small" icon={<FolderOpen size={14} />} onClick={() => void openPath(targetPath)}>
              {t("skills.openDir")}
            </Button>
          </div>
        )}
        {loading ? (
          <div className="p-12 text-center"><Spin /></div>
        ) : filteredSkills.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("skills.empty")} />
        ) : (
          <div className="flex flex-col gap-2">
            {filteredSkills.map((skill) => (
              <SkillCard
                key={`${skill.target}:${skill.sourcePath}`}
                skill={skill}
                busy={busySkill === `${skill.target}:${skill.sourcePath}`}
                onDetail={(item) => void showDetail(item)}
                onToggle={(item, enabled) => void toggleSkill(item, enabled)}
                onOpen={(item) => void openPath(item.directoryPath)}
                onUninstall={confirmUninstall}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );

  const marketplace = (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <Space.Compact className="mb-3 w-full shrink-0 px-1">
        <Select
          value={marketSource}
          className="w-32 shrink-0"
          options={[{ value: "skills.sh", label: "skills.sh" }, { value: "github", label: "GitHub" }]}
          onChange={changeMarketSource}
        />
        <Input.Search
          allowClear
          enterButton
          loading={marketplaceLoading}
          placeholder={t("skills.searchMarketplace")}
          onSearch={(query) => void searchMarketplace(query)}
        />
      </Space.Compact>
      <div
        data-testid="marketplace-list"
        className="min-h-0 flex-1 px-1"
        style={{ flex: "1 1 0%", minHeight: 0, overflowY: "auto" }}
      >
        {marketplaceLoading && marketplaceSkills.length === 0 ? (
          <div className="p-12 text-center"><Spin /></div>
        ) : marketplaceSkills.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t("skills.noResults")} />
        ) : (
          <div className="flex flex-col gap-2">
            {marketplaceSkills.map((skill) => (
              <MarketplaceCard
                key={`${skill.repo}:${skill.name}`}
                skill={skill}
                source={marketSource}
                installing={installing?.startsWith(`${skill.repo}:`) ?? false}
                installDisabled={installBusy}
                onInstall={installMarketplace}
                onOpenRepo={(repo) => void openRepository(repo)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );

  return (
    <>
      <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden" style={{ background: token.colorBgElevated }}>
        <Tabs
          className="skills-page-tabs shrink-0"
          activeKey={pageTab}
          tabBarGutter={32}
          tabBarStyle={{ padding: "0 16px", marginBottom: 0 }}
          onChange={(key) => {
            const next = key as "mine" | "market";
            setPageTab(next);
            if (next === "market" && !marketLoaded.current) void searchMarketplace("");
          }}
          items={[
            { key: "mine", label: <span className="inline-flex items-center gap-1.5"><Sparkles size={14} />{t("skills.mySkills")}</span> },
            { key: "market", label: <span className="inline-flex items-center gap-1.5"><Store size={14} />{t("skills.marketplace")}</span> },
          ]}
        />
        <div className="relative min-h-0 flex-1 overflow-hidden px-3 pb-3" style={{ flex: "1 1 0%", minHeight: 0 }}>
          <div className="relative h-full min-h-0">
            <div
              className="absolute inset-0 flex min-h-0 flex-col overflow-hidden"
              style={{ display: pageTab === "mine" ? "flex" : "none" }}
              aria-hidden={pageTab !== "mine"}
            >
              {mySkills}
            </div>
            <div
              className="absolute inset-0 flex min-h-0 flex-col overflow-hidden"
              style={{ display: pageTab === "market" ? "flex" : "none" }}
              aria-hidden={pageTab !== "market"}
            >
              {marketplace}
            </div>
          </div>
        </div>
      </div>

      <Modal
        open={Boolean(selectedSkill)}
        title={t("skills.detail")}
        centered
        destroyOnHidden
        mask={{ enabled: true }}
        width={640}
        footer={null}
        onCancel={clearSelectedSkill}
      >
        {selectedSkill && (
          <div className="select-text">
            <div className="mb-3 flex items-center gap-2">
              <Typography.Title level={4} className="!m-0">{selectedSkill.info.name}</Typography.Title>
              <Tag className="!m-0"><span className="inline-flex items-center gap-1"><SkillTargetIcon target={selectedSkill.info.target} />{t(skillTargetLabelKey(selectedSkill.info.target))}</span></Tag>
            </div>
            <Typography.Paragraph type="secondary">{selectedSkill.info.description}</Typography.Paragraph>
            <pre className="max-h-96 overflow-auto whitespace-pre-wrap rounded-lg p-4 text-[13px]" style={{ background: token.colorBgContainer }}>{selectedSkill.content}</pre>
            {selectedSkill.files.length > 0 && <Typography.Text type="secondary" className="text-xs">{t("skills.filesLabel")}: {selectedSkill.files.join(", ")}</Typography.Text>}
          </div>
        )}
      </Modal>

      <style>{`
        .skills-page-tabs .ant-tabs-nav { margin: 0; }
        .skill-target-tabs .ant-tabs-nav { margin: 0; }
        .skills-page-tabs .ant-tabs-content-holder,
        .skill-target-tabs .ant-tabs-content-holder { display: none; }
        .skill-target-tabs .ant-tabs-tab-btn { display: inline-flex; align-items: center; }
        .skill-card-hover { transition: border-color .2s; }
        .skill-card-hover:hover { border-color: ${token.colorPrimary} !important; }
        .skill-card-hover:hover .skill-card-title { color: ${token.colorPrimary} !important; }
      `}</style>
    </>
  );
}
