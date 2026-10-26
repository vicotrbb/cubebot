# Linux container isolation mechanism probes

Run date: 2026-10-26. These are preliminary Linux mechanism results for milestone 2, not G1–G3 qualification. The production Pod specification still has no runtime/tool egress filter. The tool runner remains unimplemented.

The accepted one-Pod layout needs separate runtime/tool network authority. Kubernetes NetworkPolicy selects Pods, so a shared destination rule cannot by itself distinguish these containers. Linux nftables exposes the UID associated with an originating socket, making fixed runtime/tool identities a candidate for finer enforcement. These platform facts do not establish enforcement across a particular CNI or RuntimeClass. References: [NetworkPolicy selectors](https://kubernetes.io/docs/concepts/services-networking/network-policies/#the-networkpolicy-resource) and [nftables socket UID matching](https://wiki.nftables.org/wiki-nftables/index.php/Matching_packet_metainformation#Matching_by_socket_UID_/_GID).

## Reproduce

```sh
just container-isolation
```

The script builds the runtime, tools and qualification targets from `apps/agent/Dockerfile` in a uniquely named Docker builder. It owns the builder, role images, fixture volume and containers, then removes them in its cleanup path. Cleanup attempts every owned resource, treats absence as already clean and reports unexpected failures by resource name. A cleanup failure makes an otherwise successful command fail, while an earlier probe error remains the primary exception. It never changes the current builder, Docker daemon configuration, existing workloads, host network rules, external infrastructure or a Kubernetes cluster.

The production runtime image contains the native control-loop binary as UID 10001. The tools image contains the separate tool-runner binary as UID 10002. The default Docker build selects the runtime role. The qualification target additionally contains a Rust probe, nftables and iproute2; this fixture is not copied into either production role. The image recipe builds with one Cargo job to limit memory pressure.

The fixture creates the existing 0700 runtime/tools directories using the real native storage initializer under the two UIDs and supplemental group 20000. Runtime probe code writes a fake bootstrap credential and a real private recovery journal. The tool probe receives its writable tool subpath and, deliberately, an additional ancestor-volume mount to test directory permissions even with more mount access than the production tool Pod. It attempts reads, writes and hard links without printing credential or journal content. Separate PID namespaces and shared networking match the relevant container boundaries. This does not exercise Kubernetes projected-token mounts, service-account validation or CSI behavior.

Networking uses a namespace with no external interface. IPv4 `8.8.4.4` and IPv6 `2001:4860::8888` are assigned only to its loopback device as synthetic public-address fixtures. Private loopback endpoints and these public-address fixtures use the same trusted listener. All tests use direct TCP sockets. A trusted, short-lived helper with only NET_ADMIN changes rules inside that fixture namespace. Runtime/tool probes drop all capabilities and prohibit privilege escalation. The tool probe has no kernel-policy privilege.

The finite-grant probe replaces the comparison filter atomically with IPv4/IPv6 address-and-TCP-port sets. Entries have kernel timeouts. Rules check current set membership on every outgoing tool packet, with no earlier conntrack-established acceptance rule. The runtime UID retains its separate path. A five-second entry permits a persistent socket and a fresh request/reply exchange; after a 5.2-second wait, the probe tests another exchange on that socket and a new connection. A second, thirty-second entry tests explicit deletion before its timeout. Each family also checks an ungranted port whose listener is positively reachable from the runtime. The session probe consumes its initial greeting before each requested exchange, preventing buffered greeting bytes from counting as a new reply. The fixture unit test runs in `just check`.

The kernel's per-element timeout is a candidate for finite authority without a running userspace expiry timer. It does not supply grant authentication, issuance or renewal. See [nftables element timeouts](https://wiki.nftables.org/wiki-nftables/index.php/Element_timeouts). This test only observes fresh outgoing request/reply exchanges. It does not close the socket, cancel the process, retract bytes already sent, or prove that unsolicited incoming data stops. Inbound enforcement and UDP behavior remain separate requirements.

## Recorded results

The [initial comparison report](evidence/2026-10-26-container-isolation.json) and [finite-grant report](evidence/2026-10-26-finite-network-grants.json) record the UTC timestamp, base Git revision and worktree fingerprint, local image IDs and roles, Docker/kernel/platform versions and observations.

Profile: Docker 29.4.0, Linux aarch64, kernel `7.0.14-orbstack-00380-ga7e0a2dc9535`, 3 virtual CPUs, 4,181,233,664 bytes of VM memory, nftables 1.0.6. This is a local mechanism profile, not the proposed production capacity profile.

| Probe                              | Measured observation                                                                                                                                           |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime/tool filesystem separation | Tool UID 10002 could write its own workspace. Reading the runtime credential or journal, writing the runtime directory and hard-linking its credential failed. |
| Process separation                 | Runtime and tool probes observed different PID namespace inodes.                                                                                               |
| No kernel filter                   | Tool sockets reached all four private/public IPv4/IPv6 fixtures.                                                                                               |
| Proxy environment                  | A direct tool socket still reached the IPv4 private fixture with HTTPS_PROXY set.                                                                              |
| Uniform destination filter         | Denying loopback destinations also blocked the runtime's required private path.                                                                                |
| UID-aware output filter            | Tool connections to both private fixtures failed; both public-address fixtures remained reachable. Runtime UID 10001 retained its private path.                |
| Policy modification attempt        | Tool UID 10002 received a kernel permission error when trying to flush nftables, and the private-path denial remained in effect.                               |

The finite-grant run at 2026-10-26T13:02:24Z passed for both IPv4 and IPv6 on the same platform profile. Each granted socket completed a fresh exchange. After timeout, a fresh exchange on that socket and a new connection failed. Explicit element deletion produced the same denials before the thirty-second timeout. A different, listening private port stayed inaccessible. The source fingerprint is `16fa9033a7454a55e98e885ccb8398769ff1aab21e54357fe88b69753dba0005` on baseline `1fa93f6`. This establishes the tested tuple/timeouts and outgoing exchange mechanism, not complete grant enforcement.

These observations favor a trusted setup helper with UID-aware kernel filtering over proxy-only or uniform Pod-wide enforcement for the next prototype. No helper or filter has been added to production Pod startup by this change.

## Remaining acceptance work

The fixture classifier only distinguishes the tested loopback prefixes from synthetic public addresses. Production work still needs the full private/special-use and installation-topology classifier, approved protocols and DNS paths, authenticated private-grant issuance and renewal, complete bidirectional expiry/revocation of established connections, and fail-closed initialization. Raw sockets, UDP/QUIC, redirect/resolution changes, alternate resolvers, hostile UID/namespace escape attempts, package-installed programs, browsers and MCP remain unqualified.

The setup helper's namespace-scoped NET_ADMIN privilege needs an explicit supported installation/admission profile. Credentials, mounts, user mapping, IPv6, NAT behavior, CNI policy ordering and optional sandbox runtimes must be tested in a disposable supported Kubernetes cluster. This Docker fixture does not qualify RWOP attachment, projected tokens, actual controller restart/partition recovery, fencing or arbitrary tool execution. G1–G3 remain open, and tools cannot be enabled on the strength of this report alone.
