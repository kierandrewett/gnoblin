/* Validates the lock screen's PAM auth path: the security-critical property is
 * that a WRONG password is REJECTED. Mirrors gnoblin-lock.cpp's lock_authenticate
 * exactly (same service, same conversation function). No display needed. */
#include <security/pam_appl.h>
#include <pwd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int conv(int n, const struct pam_message **msg, struct pam_response **resp, void *data) {
    const char *pw = data;
    struct pam_response *r = calloc(n, sizeof(*r));
    if (!r) return PAM_BUF_ERR;
    for (int i = 0; i < n; i++)
        if (msg[i]->msg_style == PAM_PROMPT_ECHO_OFF || msg[i]->msg_style == PAM_PROMPT_ECHO_ON)
            r[i].resp = strdup(pw ? pw : "");
    *resp = r;
    return PAM_SUCCESS;
}

static int authenticate(const char *service, const char *pw) {
    struct passwd *p = getpwuid(getuid());
    const char *user = p ? p->pw_name : "nobody";
    struct pam_conv c = { conv, (void *)pw };
    pam_handle_t *h = NULL;
    if (pam_start(service, user, &c, &h) != PAM_SUCCESS) return -1;
    int rc = pam_authenticate(h, 0);
    pam_end(h, rc);
    return rc;
}

int main(void) {
    int rc = authenticate("login", "definitely-not-the-password-9f3k2x7q");
    if (rc == PAM_SUCCESS) {
        fprintf(stderr, "FAIL: lock PAM accepted a WRONG password — insecure!\n");
        return 1;
    }
    printf("PASS: lock PAM rejected a wrong password (rc=%d: %s)\n", rc, pam_strerror(NULL, rc));
    return 0;
}
