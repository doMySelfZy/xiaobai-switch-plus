import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { App as AntdApp, ConfigProvider } from "antd";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useSiteStore } from "@/stores";
import type { Site, SiteModel } from "@/types/domain";
import { ModelPicker } from "./ModelPicker";
import i18n from "@/i18n";

function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConfigProvider>
      <AntdApp>{children}</AntdApp>
    </ConfigProvider>
  );
}

function site(lastModelFetchError: string): Site {
  return {
    id: "site-1",
    name: "Relay",
    baseUrl: "https://api.example.com",
    baseUrls: ["https://api.example.com"],
    keyPrefix: "sk-t…",
    quotaRevision: "rev-1",
    hasKey: true,
    protocol: "openai_compatible",
    claudeAuthKeyStyle: "anthropic_auth_token",
    notes: null,
    enabled: true,
    sortOrder: 0,
    selectedModelId: null,
    lastModelFetchAt: null,
    lastModelFetchLatencyMs: null,
    lastModelFetchError,
    createdAt: 1,
    updatedAt: 1,
  };
}

describe("ModelPicker fetch errors", () => {
  beforeEach(() => {
    useSiteStore.setState({ modelsBySite: {}, fetchingModels: false });
  });

  afterEach(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  it("maps a persisted unauthorized error to actionable localized copy", () => {
    render(
      <Wrapper>
        <ModelPicker site={site("unauthorized")} models={[]} />
      </Wrapper>,
    );

    expect(screen.getByText("模型列表获取失败")).toBeInTheDocument();
    expect(screen.getByText("鉴权失败，请检查 API Key 或站点协议")).toBeInTheDocument();
    expect(screen.queryByText("unauthorized")).toBeNull();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("only exposes a sanitized HTTP status as technical details", () => {
    render(
      <Wrapper>
        <ModelPicker site={site("HTTP 502 from upstream")} models={[]} />
      </Wrapper>,
    );

    expect(screen.getByText("模型列表获取失败")).toBeInTheDocument();
    expect(screen.getByText("技术详情：HTTP 502")).toBeInTheDocument();
    expect(screen.queryByText(/from upstream/)).toBeNull();
  });

  it("does not render an unknown persisted error that may contain a secret", () => {
    render(
      <Wrapper>
        <ModelPicker site={site("proxy failed for sk-super-secret")} models={[]} />
      </Wrapper>,
    );

    expect(screen.getByText("请重试或检查站点配置")).toBeInTheDocument();
    expect(screen.queryByText(/sk-super-secret/)).toBeNull();
  });

  it("renders the actionable model error in English", async () => {
    await i18n.changeLanguage("en-US");
    render(
      <Wrapper>
        <ModelPicker site={site("unauthorized")} models={[]} />
      </Wrapper>,
    );

    expect(screen.getByText("Could not fetch the model list")).toBeInTheDocument();
    expect(
      screen.getByText("Authentication failed; check the API key or site protocol"),
    ).toBeInTheDocument();
  });
});

describe("ModelPicker search", () => {
  const MODELS: SiteModel[] = [
    {
      id: "m1",
      siteId: "site-1",
      modelId: "gpt-4o",
      displayName: "GPT-4o",
      ownedBy: null,
      raw: null,
    },
    {
      id: "m2",
      siteId: "site-1",
      modelId: "claude-3-5-sonnet",
      displayName: "Claude 3.5 Sonnet",
      ownedBy: null,
      raw: null,
    },
  ];

  beforeEach(() => {
    useSiteStore.setState({ modelsBySite: {}, fetchingModels: false });
  });

  afterEach(async () => {
    await i18n.changeLanguage("zh-CN");
  });

  it("filters the chips while the input keeps the raw query", async () => {
    // 过滤走 useDeferredValue：输入框绑原始值（击键立刻回显），chip 列表可以慢一拍。
    render(
      <Wrapper>
        <ModelPicker site={{ ...site(""), selectedModelId: "gpt-4o" }} models={MODELS} />
      </Wrapper>,
    );

    expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();

    const input = screen.getByPlaceholderText("搜索模型…");
    fireEvent.change(input, { target: { value: "claude" } });

    expect(input).toHaveValue("claude");
    await waitFor(() => {
      expect(screen.queryByText("GPT-4o")).toBeNull();
    });
    expect(screen.getByText("Claude 3.5 Sonnet")).toBeInTheDocument();

    fireEvent.change(input, { target: { value: "" } });
    await waitFor(() => {
      expect(screen.getByText("GPT-4o")).toBeInTheDocument();
    });
  });
});
