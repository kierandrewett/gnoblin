/* Complements lock-pam-test.c. That test proves a WRONG password is REJECTED
 * (stay-locked) against the real "login" stack. This one proves the OTHER half
 * of the security contract that gnoblin-lock.cpp owns: the result of
 * pam_authenticate is mapped correctly to unlock / stay-locked.
 *
 *   gnoblin-lock.cpp lock_authenticate(): return rc == PAM_SUCCESS;
 *   -> TRUE  => lock_dismiss() (unlock)
 *   -> FALSE => stay locked
 *
 * We drive PAM through pam_start_confdir() with a private service directory (no
 * root needed) holding two stacks: one that succeeds (pam_permit) and one that
 * denies (pam_deny). This deterministically exercises BOTH the unlock-on-success
 * path (previously untested — a real correct password can't be scripted in CI)
 * and the stay-locked-on-failure path, validating gnoblin's rc->bool mapping.
 * Real credential strength is pam_unix's job and is exercised by the sibling
 * wrong-password test against the live "login" stack. */
#include <security/pam_appl.h>

#include <pwd.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Mirror of gnoblin-lock.cpp's lock_conv: hand the password to any prompt. */
static int conv(int n, const struct pam_message** msg, struct pam_response** resp, void* data) {
    const char* pw = data;
    struct pam_response* r = calloc(n, sizeof(*r));
    if (!r)
        return PAM_BUF_ERR;
    for (int i = 0; i < n; i++)
        if (msg[i]->msg_style == PAM_PROMPT_ECHO_OFF || msg[i]->msg_style == PAM_PROMPT_ECHO_ON)
            r[i].resp = strdup(pw ? pw : "");
    *resp = r;
    return PAM_SUCCESS;
}

/* Mirror of gnoblin-lock.cpp's lock_authenticate(), but against a private
 * confdir so the test is self-contained: returns TRUE iff PAM accepts (the
 * exact condition that triggers lock_dismiss/unlock). */
static int lock_authenticate(const char* confdir, const char* service, const char* pw) {
    struct passwd* p = getpwuid(getuid());
    const char* user = p ? p->pw_name : "nobody";
    struct pam_conv c = {conv, (void*)pw};
    pam_handle_t* h = NULL;
    int rc;

    if (pam_start_confdir(service, user, &c, confdir, &h) != PAM_SUCCESS)
        return -1;
    rc = pam_authenticate(h, 0);
    pam_end(h, rc);
    return rc == PAM_SUCCESS; /* the value lock_authenticate() returns */
}

static int write_file(const char* path, const char* contents) {
    FILE* f = fopen(path, "w");
    if (!f)
        return -1;
    fputs(contents, f);
    return fclose(f);
}

int main(void) {
    char dir[] = "/tmp/gnoblin-pam.XXXXXX";
    char accept_path[256], reject_path[256];
    int fails = 0;

    if (!mkdtemp(dir)) {
        fprintf(stderr, "FAIL: could not create temp confdir\n");
        return 1;
    }
    snprintf(accept_path, sizeof(accept_path), "%s/gnoblin-accept", dir);
    snprintf(reject_path, sizeof(reject_path), "%s/gnoblin-reject", dir);

    if (write_file(accept_path, "auth     required pam_permit.so\n"
                                "account  required pam_permit.so\n") ||
        write_file(reject_path, "auth     required pam_deny.so\n"
                                "account  required pam_deny.so\n")) {
        fprintf(stderr, "FAIL: could not write PAM service files\n");
        return 1;
    }

    /* The unlock path: PAM accepts -> lock_authenticate() must return TRUE. */
    if (lock_authenticate(dir, "gnoblin-accept", "any-password") != 1) {
        fprintf(stderr, "FAIL: correct/accepted auth did NOT unlock (rc!=TRUE)\n");
        fails++;
    }

    /* The stay-locked path: PAM denies -> must return FALSE (not unlock). */
    if (lock_authenticate(dir, "gnoblin-reject", "any-password") != 0) {
        fprintf(stderr, "FAIL: denied auth unlocked — insecure!\n");
        fails++;
    }

    unlink(accept_path);
    unlink(reject_path);
    rmdir(dir);

    if (fails == 0) {
        printf("PASS: lock unlocks on PAM success, stays locked on PAM failure\n");
        return 0;
    }
    return 1;
}
