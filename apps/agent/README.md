# Agent application

One Rust package owns each agent's runtime and tool-runner implementation:

| Executable            | Entry point              | Intended process                                                          |
| --------------------- | ------------------------ | ------------------------------------------------------------------------- |
| `cubebot-agent`       | `src/main.rs`            | Runtime container: agent loop, provider calls, orchestrator communication |
| `cubebot-tool-runner` | `src/bin/tool-runner.rs` | Tool container: authorized browser, terminal, and MCP execution           |

Normal runtime execution runs the authenticated identity/control loop, synchronizes recovery evidence and acknowledges safe drain. Turn admission, inference and tool invocation remain unavailable. The tool runner prints an explicit unimplemented message and exits unsuccessfully. The runtime also supports the trusted init-container mode `cubebot-agent --initialize-storage runtime|tools`. It initializes the corresponding fixed directory beneath `/storage`, validates ownership/type, restores private permissions and synchronizes directory metadata while retaining existing contents. The pod compiler invokes it with the matching non-root identity before application containers start. This local operation does not implement agent work, arbitrary commands, browser, provider requests or authentication.

The separation is between execution environments, not top-level applications. ADR 0033 requires keeping provider credentials and runtime authority out of the tool process. A shared source package does not establish that isolation; container construction, invocation authentication, and recovery still need implementation and qualification.

The internal modules own agent-specific behavior. Durable records, shared skill-library revisions, connection refresh, and installation permissions remain orchestrator responsibilities. Shared provider adapters and communication types belong under `modules/`.

The trusted network setup mode is `cubebot-agent --initialize-network '<JSON>'`. The non-secret argument requires `infrastructure`, an array of canonical CIDRs, and `dns`, up to four resolver IPs. Input is limited to 16 KiB and 128 infrastructure prefixes. This command requires NET_ADMIN in the intended isolated network namespace and the image's `/usr/sbin/nft`; it must complete before any tool starts. It never reads stdin. The nft batch has a five-second deadline and atomically replaces only `inet cubebot_network`. Do not run it in the host network namespace.

The baseline permits public TCP/UDP, restricts special-use and configured infrastructure addresses, permits configured DNS resolvers only on TCP/UDP port 53, and keeps runtime UID 10001 separate from tool UID 10002. Other UIDs and forwarded traffic are denied. IPv6 neighbor discovery and related ICMP control traffic have explicit kernel rules. IPv6 public scope is currently 2000::/3 minus restricted prefixes; NAT64, 6to4 and other special destinations are not public exceptions. Installation topology must include infrastructure exposed on public addresses and nonstandard translation ranges.

This command does not issue private grants or authenticate invocations. Production Pod startup is not wired to it yet. See the [isolation evidence](../../docs/development/container-isolation.md) for tested mechanisms and remaining CNI, storage, hostile-tool and fencing qualification.
