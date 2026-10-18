# Unsigned adapter-only native observation

Bundle: `dev.hmrguez.gitjuke.alerts.qualification`. No account, database, credential or provider data. The normal app was not launched or enabled by this task.

1. Build `scripts/qualify-notification-adapter.sh`; launch only its separate AppKit helper.
2. Observe actual native AX and screenshot: permission 1 / not determined, no automatic permission request.
3. Activate Send: adapter log records refusal before permission; no OS dispatch occurred.
4. Activate Enable once: log records explicit request; OS authorization remained not determined during observation. No granted/denied result or prompt-display success is claimed.
5. Proposed ad-hoc signing was rejected by automatic approval review. The user chose **Keep signed-native verification blocked**. Signing was not performed.
6. Capture AX/screenshot/log, then terminate only the helper.

These observations are real native adapter/OS-status evidence with synthetic text, not a native reader delivery/click flow. The observed binary predates the final bounded OS settings polling/request-in-flight guard. Final source compiles and links through the desktop IPC test. Positive OS permission, delivery, click, cold activation and native category/privacy/recovery matrices remain Blocked/Pending. No live GitHub or full release qualification follows.
