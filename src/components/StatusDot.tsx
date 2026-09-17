import { Badge, theme } from "antd";
import { clsx } from "clsx";

type StatusType = "available" | "unavailable" | "disabled";

interface Props {
  /** @deprecated Use `status` instead for three-state control */
  active?: boolean;
  /** Status type: available (green pulsing), unavailable (red static), disabled (gray static) */
  status?: StatusType;
  title?: string;
  className?: string;
}

/** 
 * Status indicator dot with three states:
 * - available: Green pulsing badge (enabled & working)
 * - unavailable: Red static badge (enabled but not working)
 * - disabled: Gray static badge (disabled)
 * 
 * Legacy `active` prop is still supported for backward compatibility.
 */
export function StatusDot({ active, status, title, className }: Props) {
  const { token } = theme.useToken();

  // Legacy support: convert `active` to `status`
  const finalStatus: StatusType = status ?? (active ? "available" : "disabled");

  const badgeConfig = {
    available: {
      status: "processing" as const,
      color: token.colorSuccess,
      dataStatus: "available",
    },
    unavailable: {
      status: "default" as const,
      color: token.colorError,
      dataStatus: "unavailable",
    },
    disabled: {
      status: "default" as const,
      color: token.colorTextQuaternary,
      dataStatus: "disabled",
    },
  }[finalStatus];

  return (
    <Badge
      className={clsx("status-dot inline-flex shrink-0", className)}
      status={badgeConfig.status}
      color={badgeConfig.color}
      title={title}
      aria-hidden
      data-status={badgeConfig.dataStatus}
    />
  );
}
