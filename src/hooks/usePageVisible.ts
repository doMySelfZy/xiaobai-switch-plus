import { useEffect, useState } from "react";
import { useUIStore, type AppPage } from "@/stores";

/**
 * 页面真正可见：当前页就是它，且窗口没被最小化/切到后台。
 *
 * KeepAlivePages 让访问过的页面一直挂着（只是 display:none），所以定时器不能只判
 * `document.visibilityState` —— 那只能说明窗口可见，不代表这个页面在看。只看窗口可见性
 * 会让隐藏页面继续打网络请求。
 */
export function usePageVisible(page: AppPage): boolean {
  const activePage = useUIStore((s) => s.activePage);
  const [windowVisible, setWindowVisible] = useState(
    () => typeof document === "undefined" || document.visibilityState === "visible",
  );

  useEffect(() => {
    const onChange = () => setWindowVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);

  return activePage === page && windowVisible;
}
