#define _GNU_SOURCE
#include "gnoblin-vr-input.h"

#include <string.h>

struct VrInput {
    XrInstance instance;
    XrActionSet set;
    XrAction aim_pose, select, grab, menu, scroll;
    XrPath hand_path[2];
    XrSpace aim_space[2];
};

static XrPath path_of(XrInstance instance, const char* text) {
    XrPath path = XR_NULL_PATH;
    xrStringToPath(instance, text, &path);
    return path;
}

static XrAction make_action(VrInput* input, const char* name, const char* localised,
                            XrActionType type) {
    XrActionCreateInfo info = {XR_TYPE_ACTION_CREATE_INFO};
    g_strlcpy(info.actionName, name, sizeof info.actionName);
    g_strlcpy(info.localizedActionName, localised, sizeof info.localizedActionName);
    info.actionType = type;
    info.countSubactionPaths = 2;
    info.subactionPaths = input->hand_path;
    XrAction action = XR_NULL_HANDLE;
    xrCreateAction(input->set, &info, &action);
    return action;
}

/* Where each profile keeps the controls the desktop uses. NULL means the profile has none. */
typedef struct {
    const char* profile;
    const char* select;
    const char* grab;
    const char* scroll;
    const char* menu;
    gboolean menu_left_only; /* Quest controllers have a menu button on the left hand only */
} Profile;

static const Profile profiles[] = {
    {"/interaction_profiles/khr/simple_controller", "select/click", NULL, NULL, "menu/click",
     FALSE},
    {"/interaction_profiles/oculus/touch_controller", "trigger/value", "squeeze/value",
     "thumbstick", "menu/click", TRUE},
    {"/interaction_profiles/meta/touch_controller_plus", "trigger/value", "squeeze/value",
     "thumbstick", "menu/click", TRUE},
    {"/interaction_profiles/meta/touch_pro_controller", "trigger/value", "squeeze/value",
     "thumbstick", "menu/click", TRUE},
    {"/interaction_profiles/valve/index_controller", "trigger/value", "squeeze/value", "thumbstick",
     "b/click", FALSE},
    {"/interaction_profiles/htc/vive_controller", "trigger/click", "squeeze/click", NULL,
     "menu/click", FALSE},
};

static gboolean suggest(VrInput* input, const Profile* profile) {
    XrActionSuggestedBinding bindings[16];
    guint count = 0;
    static const char* const hands[2] = {"left", "right"};
    for (int h = 0; h < 2; h++) {
        g_autofree char* aim = g_strdup_printf("/user/hand/%s/input/aim/pose", hands[h]);
        bindings[count++] =
            (XrActionSuggestedBinding){input->aim_pose, path_of(input->instance, aim)};
        const struct {
            XrAction action;
            const char* suffix;
        } controls[] = {{input->select, profile->select},
                        {input->grab, profile->grab},
                        {input->scroll, profile->scroll},
                        {input->menu, (!profile->menu_left_only || h == 0) ? profile->menu : NULL}};
        for (guint c = 0; c < G_N_ELEMENTS(controls); c++) {
            if (!controls[c].suffix)
                continue;
            g_autofree char* path =
                g_strdup_printf("/user/hand/%s/input/%s", hands[h], controls[c].suffix);
            bindings[count++] =
                (XrActionSuggestedBinding){controls[c].action, path_of(input->instance, path)};
        }
    }
    XrInteractionProfileSuggestedBinding suggested = {
        XR_TYPE_INTERACTION_PROFILE_SUGGESTED_BINDING};
    suggested.interactionProfile = path_of(input->instance, profile->profile);
    suggested.suggestedBindings = bindings;
    suggested.countSuggestedBindings = count;
    return xrSuggestInteractionProfileBindings(input->instance, &suggested) == XR_SUCCESS;
}

VrInput* vr_input_new(XrInstance instance, XrSession session) {
    VrInput* input = g_new0(VrInput, 1);
    input->instance = instance;
    XrActionSetCreateInfo set_info = {XR_TYPE_ACTION_SET_CREATE_INFO};
    g_strlcpy(set_info.actionSetName, "desktop", sizeof set_info.actionSetName);
    g_strlcpy(set_info.localizedActionSetName, "Desktop", sizeof set_info.localizedActionSetName);
    if (xrCreateActionSet(instance, &set_info, &input->set) != XR_SUCCESS) {
        g_free(input);
        return NULL;
    }
    input->hand_path[0] = path_of(instance, "/user/hand/left");
    input->hand_path[1] = path_of(instance, "/user/hand/right");
    input->aim_pose = make_action(input, "aim", "Aim", XR_ACTION_TYPE_POSE_INPUT);
    input->select = make_action(input, "select", "Select", XR_ACTION_TYPE_BOOLEAN_INPUT);
    input->grab = make_action(input, "grab", "Grab", XR_ACTION_TYPE_BOOLEAN_INPUT);
    input->menu = make_action(input, "menu", "Menu", XR_ACTION_TYPE_BOOLEAN_INPUT);
    input->scroll = make_action(input, "scroll", "Scroll", XR_ACTION_TYPE_VECTOR2F_INPUT);
    guint accepted = 0;
    for (guint i = 0; i < G_N_ELEMENTS(profiles); i++)
        accepted += suggest(input, &profiles[i]) ? 1 : 0;
    if (!accepted) {
        vr_input_free(input);
        return NULL;
    }
    XrSessionActionSetsAttachInfo attach = {XR_TYPE_SESSION_ACTION_SETS_ATTACH_INFO};
    attach.countActionSets = 1;
    attach.actionSets = &input->set;
    if (xrAttachSessionActionSets(session, &attach) != XR_SUCCESS) {
        vr_input_free(input);
        return NULL;
    }
    for (int h = 0; h < 2; h++) {
        XrActionSpaceCreateInfo space_info = {XR_TYPE_ACTION_SPACE_CREATE_INFO};
        space_info.action = input->aim_pose;
        space_info.subactionPath = input->hand_path[h];
        space_info.poseInActionSpace.orientation.w = 1.0f;
        xrCreateActionSpace(session, &space_info, &input->aim_space[h]);
    }
    return input;
}

void vr_input_free(VrInput* input) {
    if (!input)
        return;
    for (int h = 0; h < 2; h++)
        if (input->aim_space[h])
            xrDestroySpace(input->aim_space[h]);
    if (input->set)
        xrDestroyActionSet(input->set);
    g_free(input);
}

void vr_input_sync(VrInput* input, XrSession session, XrSpace space, XrTime time, VrHand hands[2]) {
    memset(hands, 0, 2 * sizeof(VrHand));
    XrActiveActionSet active = {input->set, XR_NULL_PATH};
    XrActionsSyncInfo sync = {XR_TYPE_ACTIONS_SYNC_INFO};
    sync.countActiveActionSets = 1;
    sync.activeActionSets = &active;
    if (xrSyncActions(session, &sync) != XR_SUCCESS)
        return;
    for (int h = 0; h < 2; h++) {
        XrSpaceLocation location = {XR_TYPE_SPACE_LOCATION};
        if (xrLocateSpace(input->aim_space[h], space, time, &location) == XR_SUCCESS &&
            (location.locationFlags & XR_SPACE_LOCATION_POSITION_VALID_BIT) &&
            (location.locationFlags & XR_SPACE_LOCATION_ORIENTATION_VALID_BIT)) {
            hands[h].active = TRUE;
            hands[h].aim = (VrPose){
                {location.pose.position.x, location.pose.position.y, location.pose.position.z},
                {location.pose.orientation.x, location.pose.orientation.y,
                 location.pose.orientation.z, location.pose.orientation.w}};
        }
        XrActionStateGetInfo get = {XR_TYPE_ACTION_STATE_GET_INFO};
        get.subactionPath = input->hand_path[h];
        XrActionStateBoolean state = {XR_TYPE_ACTION_STATE_BOOLEAN};
        get.action = input->select;
        if (xrGetActionStateBoolean(session, &get, &state) == XR_SUCCESS && state.isActive)
            hands[h].select = state.currentState;
        get.action = input->grab;
        if (xrGetActionStateBoolean(session, &get, &state) == XR_SUCCESS && state.isActive)
            hands[h].grab = state.currentState;
        get.action = input->menu;
        if (xrGetActionStateBoolean(session, &get, &state) == XR_SUCCESS && state.isActive)
            hands[h].menu = state.currentState;
        XrActionStateVector2f stick = {XR_TYPE_ACTION_STATE_VECTOR2F};
        get.action = input->scroll;
        if (xrGetActionStateVector2f(session, &get, &stick) == XR_SUCCESS && stick.isActive) {
            hands[h].scroll[0] = stick.currentState.x;
            hands[h].scroll[1] = stick.currentState.y;
        }
    }
}
