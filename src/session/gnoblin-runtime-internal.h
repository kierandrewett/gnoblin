#ifndef GNOBLIN_RUNTIME_INTERNAL_H
#define GNOBLIN_RUNTIME_INTERNAL_H

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include "../control/gnoblin-runtime-protocol.h"
#include "../input/gnoblin-input-config.h"
#include "gnoblin-runtime-spawn.h"
#include "gnoblin-priority.h"
#include "gnoblin-autostart.h"
#include "gnoblin-session-class.h"
#include "../config/gnoblin-config.h"
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <signal.h>
#include <stdarg.h>
#include <stdlib.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <linux/memfd.h>
#include <sys/wait.h>
#include <unistd.h>
#include <glib-unix.h>
#include <glib/gstdio.h>
#include <gio/gdesktopappinfo.h>
#define RUNTIME_FD 198
#define WORKER_READY_FD 199
#define WORKER_HOST_FD 200
#define GUARDIAN_STATUS_FD 201
#define GUARDIAN_AUTOSTART_FD 202
#define WORKER_GUARDIAN_STATUS_FD 203
#define WORKER_RECOVERY_SNAPSHOT_FD 204
#define STARTUP_TIMEOUT_MS 10000
/* The first Lua worker must send its configuration within this much CPU time.
 * A loop in a config burns CPU and is stopped quickly. A worker that the
 * kernel has not scheduled yet, on a loaded boot, is not. The wall limit stops
 * a worker that is stuck without using CPU. */
#define WORKER_CONFIG_CPU_TIMEOUT_MS 3000
#define WORKER_CONFIG_WALL_TIMEOUT_MS 30000
#define SUSPEND_TIMEOUT_MS 10000
#define WORKER_MAX_RESTARTS 5
#define WORKER_STABLE_RESET_MS 60000
#define EXIT_RESUME_REJECTED 75
#define EXIT_POWER_SUSPEND 80
#define EXIT_POWER_HIBERNATE 81
#define EXIT_POWER_REBOOT 82
#define EXIT_POWER_POWEROFF 83
#define EXIT_CONFIG_RESTORE 84
#define GUARDIAN_STATUS_STARTED 1
#define GUARDIAN_STATUS_READY 2
#define GUARDIAN_STATUS_LOGOUT 3
#define GUARDIAN_STATUS_SESSION_LIFECYCLE_SUPPORTED 4
#define SESSION_SUPERVISOR_BUS_NAME "org.gnoblin.SessionSupervisor"
#define PORTAL_BACKEND_BUS_NAME "org.freedesktop.impl.portal.desktop.gnoblin"
#define PORTAL_LIFECYCLE_INTERFACE "org.gnoblin.Portal.InhibitLifecycle"
#define SESSION_DIAGNOSTIC_MAX_BYTES (64 * 1024)
#ifndef GNOBLIN_DEFAULT_COMPOSITOR
#define GNOBLIN_DEFAULT_COMPOSITOR "/usr/bin/gnoblin"
#endif
typedef struct {
    GMainLoop* loop;
    GnoblinRuntimeWriter* writer;
    GnoblinRuntimeReader* reader;
    GHashTable* state_revisions;
    GHashTable* pending_logout_ids;
    GHashTable* pending_operation_ids;
    GQueue policy_events;
    GQueue deferred_packets;
    struct _RuntimeReload* pending_reload;
    guint reload_progress_id;
    guint deferred_packets_idle_id;
    guint deferred_callback_idle_id;
    guint64 event_sequence;
    guint64 current_client_id;
    /* Nonzero only while draining operations emitted by one INPUT callback. */
    guint64 active_input_request_id;
    GPid compositor_pid;
    int compositor_stderr_fd;
    int session_log_fd;
    int channel_fd;
    gboolean ready;
    gboolean failed;
    gboolean logout_requested;
    gboolean resume_worker;
    gboolean send_initial_autostart;
    gboolean config_fallback;
    gboolean startup_config_pending;
    gboolean startup_saved_pending;
    char* startup_failure;
    guint startup_config_idle_id;
    gboolean config_fallback_last_good;
    const char* config_fallback_error;
    gboolean resume_rejected;
    gboolean recovery_snapshot;
    GVariant* recovery_runtime_events;
    int ready_fd;
    int host_control_fd;
    int guardian_status_fd;
    GVariant* initial_document;
    GVariant* session_environment;
    guint startup_timeout_id;
    guint child_watch_id;
    guint write_watch_id;
    guint policy_event_idle_id;
    int signal_exit_status;
    gint exit_status;
} Runtime;
typedef struct {
    const char* event;
    GVariant* payload;
} RuntimePolicyEvent;
typedef struct _RuntimeReload {
    guint64 request_id;
    guint64 revision;
    guint64 generation;
    char* path;
    GVariant* document;
    gboolean portal_routes_changed;
    guint timeout_id;
    gboolean sent;
    gboolean startup;
    gboolean saved_fallback;
    /* Messages for settings that were ignored because they are invalid. NULL when nothing was
     * ignored. */
    GPtrArray* ignored;
} RuntimeReload;
typedef struct {
    GnoblinRuntimePacketType type;
    guint64 request_id;
    GVariant* payload;
} RuntimeDeferredPacket;
typedef struct {
    Runtime* runtime;
    int signal_number;
} RuntimeSignal;
typedef struct {
    GPid pid;
    GPid process_group;
    char* name;
    gboolean leader_reaped;
} HostAutostartChild;
typedef struct {
    char* name;
    char** argv;
} HostAutostartEntry;

/* Declarations shared between the split files. */
void rotate_session_diagnostic(const char* current, const char* previous);
int open_session_diagnostic(const char* name);
void session_diagnostic_event(Runtime* runtime, const char* format, ...);
void session_diagnostic_wait_status(Runtime* runtime, const char* role, GPid pid, int status);
void drain_compositor_stderr(Runtime* runtime, int log_fd);
char* config_recovery_notice(void);
void append_config_notice(const char* diagnostic);
void append_config_warning_notice(const char* diagnostic);
void set_config_fallback_marker(gboolean enabled, gboolean last_good, const char* diagnostic);
extern gint64 guardian_started_us;
void record_compositor_crash(int signal_number, int exit_status);
void discard_stale_crash_notice(void);
void report_previous_crash(void);
void set_reload_failure_notice(Runtime* runtime, const char* diagnostic);
gboolean restart_active_portal_service(GError** error);
GVariant* startup_fallback_document(const char* path, const char* failure,
                                    gboolean* used_last_good);
GDBusConnection* claim_session_lifecycle_bus(void);
void notify_portal_session_ending(GDBusConnection* connection);
gboolean prepare_session_environment(GnoblinSessionClass session_class);
gboolean activate_session(GnoblinSessionClass session_class);
char* gnoblin_installed_prefix(void);
#define GNOBLIN_GREETER_SYSTEM_CONFIG "/etc/gnoblin/greeter.lua"
void runtime_fail(Runtime* runtime, const char* message);
GVariant* resume_packet_payload(GVariant* document, guint64 revision, guint64 generation,
                                guint64 operation_id_watermark, GVariant* saved_events);
gboolean is_pending_logout(Runtime* runtime, guint64 request_id);
gboolean send_autostart_snapshot(int fd, GVariant* document, GVariant* environment, GError** error);
gboolean send_config(Runtime* runtime, GVariant* document, guint64 revision, guint64 generation,
                     guint64 transaction_id, GError** error);
void runtime_reload_free(RuntimeReload* reload);
void runtime_deferred_packet_free(gpointer data);
gboolean send_api_response(Runtime* runtime, guint64 request_id, GVariant* result,
                           const char* message, GError** error);
void runtime_signal_ready(Runtime* runtime);
void runtime_reload_finish(Runtime* runtime, gboolean commit, const char* message);
gboolean runtime_reload_timeout(gpointer user_data);
void runtime_schedule_reload_progress(Runtime* runtime);
gboolean queue_packet(Runtime* runtime, GnoblinRuntimePacketType type, guint64 request_id,
                      GVariant* payload, GError** error);
gboolean flush_startup_packet(Runtime* runtime, GError** error);
gboolean send_pending_operations(Runtime* runtime, GError** error);
gboolean handle_event(Runtime* runtime, GVariant* payload, GError** error);
gboolean handle_input(Runtime* runtime, guint64 request_id, GVariant* payload, GError** error);
gboolean handle_completion(Runtime* runtime, guint64 packet_id, GVariant* payload, GError** error);
void runtime_schedule_deferred_callbacks(gpointer user_data);
void runtime_policy_event_free(gpointer data);
void focus_policy_committed(GVariant* policy, guint64 revision, gpointer user_data);
void dispatch_reload_event(Runtime* runtime, const char* event, const char* path, guint64 revision,
                           const char* failure_message);
gboolean handle_state(Runtime* runtime, GVariant* payload, GError** error);
gboolean handle_api_request(Runtime* runtime, guint64 request_id, GVariant* payload,
                            GError** error);
gboolean runtime_load_startup_config(gpointer user_data);
gboolean startup_timeout(gpointer user_data);
gboolean runtime_signal_received(gpointer user_data);
void runtime_schedule_deferred_packets(Runtime* runtime);
gboolean runtime_channel_ready(gint fd, GIOCondition condition, gpointer user_data);
void terminate_and_reap(Runtime* runtime);
gboolean host_reap_compositor(Runtime* runtime);
GPid spawn_compositor(const char* path, int parent_fd, int child_fd, int stderr_fd,
                      gboolean xwayland, gboolean devkit, const char* wayland_display,
                      GError** error);
void usage(const char* program);
GVariant* read_recovery_snapshot(int fd, GError** error);
int runtime_worker_main(int argc, char** argv);
extern volatile sig_atomic_t host_signal_number;
void host_signal_handler(int signal_number);
gboolean move_fd_above_runtime_targets(int fd, int* moved_fd, GError** error);
int create_recovery_snapshot_fd(GVariant* snapshot, GError** error);
GPid spawn_runtime_worker(const char* executable, const char* config_path, gboolean devkit,
                          const char* wayland_display, gboolean resume, gboolean send_autostart,
                          const char* startup_recovery, GVariant* recovery_snapshot,
                          guint64 settings_revision, guint64 runtime_generation,
                          guint64 operation_id_watermark, int channel_fd, int host_control_fd,
                          int guardian_status_fd, int* ready_fd, GError** error);
gboolean drain_worker_ready_fd(int fd, gboolean* start_packet_sent);
gboolean wait_for_session_state_published(int channel_fd, guint64 revision);
gboolean guardian_set_session_lifecycle_state(int channel_fd, const char** current_state,
                                              guint64* revision, const char* state);
gboolean send_worker_disconnected(int channel_fd, GError** error);
void notify_runtime_recovery_failed(int channel_fd);
gboolean wait_for_worker_suspended(int channel_fd, Runtime* host, guint64* settings_revision,
                                   guint64* runtime_generation, guint64* operation_id_watermark,
                                   GVariant** snapshot, GError** error);
gboolean sleep_before_worker_restart(guint restart_number, Runtime* host);
void host_autostart_child_free(gpointer data);
gboolean reconcile_autostart(GVariant* entries, GVariant* environment, GPtrArray* children,
                             GPtrArray** active_entries, GError** error);
void start_xdg_autostart(GVariant* environment);
gboolean guardian_send_status(int fd, guint8 status);
gboolean forward_autostart_to_guardian(int fd, GVariant* entries, GVariant* environment,
                                       GError** error);
void reap_autostart_children(GPtrArray* children);
void stop_autostart_children(GPtrArray* children);
GPid spawn_session_supervisor(const char* executable, const char* config_path, gboolean devkit,
                              const char* wayland_display, int runtime_fd, int status_parent_fd,
                              int status_child_fd, int autostart_parent_fd, int autostart_child_fd,
                              gboolean resume, guint64 settings_revision,
                              guint64 runtime_generation, guint64 operation_id_watermark,
                              gboolean autostart_complete, GVariant* recovery_snapshot,
                              GError** error);
int session_supervisor_main(int argc, char** argv);

#endif /* GNOBLIN_RUNTIME_INTERNAL_H */
