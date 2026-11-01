import { ParaglideMessage } from "@inlang/paraglide-js-react";
import { Clock, TriangleAlert } from "lucide-react";
import type { ReactNode } from "react";

import { RelativeTime } from "@/components/relative-time";
import { Badge } from "@/components/shadcn/badge";
import { isExpired } from "@/features/machines/lib/utils";
import type {
  ConnectivityResponse,
  EnrollmentResponse,
  Protocol,
  ProtocolsResponse,
  SshAuthMethodResponse,
} from "@/generated/api/types.gen";
import { zProtocol } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { useNow } from "@/hooks/now";
import { PROTOCOL_NAMES } from "@/lib/protocols";

export function MachineConnectionBadge({
  connectivity,
}: {
  connectivity: ConnectivityResponse;
}) {
  switch (connectivity.kind) {
    case "direct":
      return (
        <div className="flex items-center gap-2">
          <Badge variant="secondary">{m.machine_connectivity_direct()}</Badge>
          <span className="font-mono text-xs text-muted-foreground tabular-nums">
            {connectivity.host}
          </span>
        </div>
      );
    case "tunneled":
      return (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <Badge variant="secondary">{m.machine_connectivity_tunneled()}</Badge>
          <MachineEnrollmentBadge enrollment={connectivity.enrollment} />
        </div>
      );
  }
}

export function MachineEnrollmentBadge({
  enrollment,
}: {
  enrollment: EnrollmentResponse;
}) {
  const expired = useNow((now) => isExpired(enrollment, now));
  const current = expired ? { state: "expired" as const } : enrollment;

  switch (current.state) {
    case "not_enrolled":
      return (
        <Badge variant="outline" className="text-warning">
          <TriangleAlert />
          {m.tunnel_state_not_set_up()}
        </Badge>
      );
    case "expired":
      return (
        <Badge variant="outline" className="text-warning">
          <TriangleAlert />
          {m.tunnel_state_expired()}
        </Badge>
      );
    case "invited":
      return (
        <>
          <Badge variant="outline" className="text-warning">
            <Clock />
            {m.tunnel_state_pending()}
          </Badge>
          <span className="text-xs text-muted-foreground">
            <ParaglideMessage
              message={m.tunnel_state_pending_expiry}
              markup={{
                expiry: () => <RelativeTime iso={current.expires_at} />,
              }}
            />
          </span>
        </>
      );
    case "enrolled":
      return (
        <span className="text-xs text-muted-foreground">
          {m.tunnel_state_registered()}
        </span>
      );
  }
}

type ProtocolBadges = {
  [P in Protocol]: (settings: NonNullable<ProtocolsResponse[P]>) => ReactNode;
};

const PROTOCOL_BADGES: ProtocolBadges = {
  ssh: (ssh) => (
    <MachineAuthenticationBadge authentication={ssh.authentication.method} />
  ),
};

function protocolBadge<P extends Protocol>(
  protocol: P,
  settings: NonNullable<ProtocolsResponse[P]>,
) {
  return (
    <div key={protocol} className="flex items-center gap-1.5">
      <Badge variant="secondary">{PROTOCOL_NAMES[protocol]}</Badge>
      {PROTOCOL_BADGES[protocol](settings)}
    </div>
  );
}

export function MachineProtocolsBadge({
  protocols,
}: {
  protocols: ProtocolsResponse;
}) {
  const badges = zProtocol.options.flatMap((protocol) => {
    const settings = protocols[protocol];
    return settings ? [protocolBadge(protocol, settings)] : [];
  });

  if (badges.length === 0)
    return (
      <span className="text-xs text-muted-foreground">
        {m.machine_protocols_none()}
      </span>
    );

  return <div className="flex flex-wrap items-center gap-2">{badges}</div>;
}

export function MachineAuthenticationBadge({
  authentication,
}: {
  authentication: SshAuthMethodResponse;
}) {
  switch (authentication.kind) {
    case "password":
      return (
        <Badge variant="outline">
          <span className="text-muted-foreground">
            {m.machine_authentication_password()}
          </span>
        </Badge>
      );
    case "keypair":
      return (
        <Badge variant="outline">
          <span className="text-muted-foreground">
            {authentication.source === "managed"
              ? m.machine_authentication_managed_key()
              : m.machine_authentication_imported_key()}
          </span>
        </Badge>
      );
  }
}
