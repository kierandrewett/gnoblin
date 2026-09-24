#include "gnoblin-config.h"
#include <glib/gstdio.h>

static GVariant* evaluate(const char* path, const char* source) {
    g_autoptr(GError) error = NULL;
    g_assert_true(g_file_set_contents(path, source, -1, &error));
    GVariant* result = gnoblin_config_load_document(path, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(result);
    return result;
}

/* Dictionary order is unspecified; array order is part of the contract. */
static void assert_equal(GVariant* actual, GVariant* expected) {
    g_assert_nonnull(actual);
    g_assert_true(g_variant_is_of_type(actual, g_variant_get_type(expected)));
    if (!g_variant_is_container(expected)) {
        g_assert_true(g_variant_equal(actual, expected));
        return;
    }
    g_assert_cmpuint(g_variant_n_children(actual), ==, g_variant_n_children(expected));
    if (g_variant_is_of_type(expected, G_VARIANT_TYPE_VARDICT)) {
        GVariantIter iter;
        const char* key;
        GVariant* value;
        g_variant_iter_init(&iter, expected);
        while (g_variant_iter_next(&iter, "{&sv}", &key, &value)) {
            g_autoptr(GVariant) found = g_variant_lookup_value(actual, key, NULL);
            assert_equal(found, value);
            g_variant_unref(value);
        }
    } else {
        for (gsize i = 0; i < g_variant_n_children(expected); i++) {
            g_autoptr(GVariant) a = g_variant_get_child_value(actual, i);
            g_autoptr(GVariant) b = g_variant_get_child_value(expected, i);
            assert_equal(a, b);
        }
    }
}

int main(void) {
    g_autoptr(GError) error = NULL;
    g_autofree char* directory = g_dir_make_tmp("gnoblin-api-XXXXXX", &error);
    g_assert_no_error(error);
    g_autofree char* root = g_build_filename(directory, "init.lua", NULL);
    g_autofree char* component = g_build_filename(directory, "component.lua", NULL);
    g_autoptr(GVariant) imported = evaluate(
        component,
        "return {shortcuts={{name='terminal',binding='<Super>Return',command={'old','arg'}},"
        "{name='keep',binding='<Super>k',command={'keep'}}},"
        "['window-rules']={{match={type='window'},opacity=1}}}\n");
    const char* source = "assert(require('gnoblin') == gnoblin)\n"
                         "gnoblin.load('component.lua')\n"
                         "gnoblin.configure {shell={minimize_duration=150},"
                         "layer_shell={preserve_active_window=true},"
                         "window_management={constrain_drag_to_work_area=true,workspace_names={'Main','Chat'}},"
                         "compositor={enable_animations=false,visual_bell=true},"
                         "input={orientation_lock=true,keyboard={xkb_options={'caps:escape'}}},"
                         "input_sources={sources={{type='xkb',id='us'}},per_window=false},"
                         "frame_renderers={my_frame={'my_renderer'}}}\n"
                         "gnoblin.shortcut {name='terminal',command={'new'}}\n"
                         "gnoblin.shortcut {name='temporary',command={'unused'}}\n"
                         "gnoblin.remove_shortcut('temporary')\n"
                         "gnoblin.remove_shortcut('missing')\n"
                         "gnoblin.autostart {name='bar',command={'old'}}\n"
                         "gnoblin.autostart {name='bar',command={'waybar'}}\n"
                         "gnoblin.autostart {name='remove',command={'unused'}}\n"
                         "gnoblin.remove_autostart('remove')\n"
                         "local rule={match={app_id='my_app',focused=false},opacity=0.95,"
                         "shader_uniforms={my_strength=0.5},corners={keep_maximized=true}}\n"
                         "gnoblin.window_rule(rule)\n"
                         "rule.opacity=0.1\n"
                         "assert(rule.match.app_id == 'my_app')\n"
                         "gnoblin.permission_rule {name='capture',match='^app-id:my_app$',"
                         "capabilities={'screen-cast'},level='ask'}\n";
    g_autoptr(GVariant) actual = evaluate(root, source);
    g_autoptr(GVariant) expected =
        evaluate(root, "return {shell={['minimize-duration']=150},"
                       "['layer-shell']={['preserve-active-window']=true},"
                       "['window-management']={['constrain-drag-to-work-area']=true,"
                       "['workspace-names']={'Main','Chat'}},"
                       "compositor={['enable-animations']=false,['visual-bell']=true},"
                       "input={['orientation-lock']=true,keyboard={['xkb-options']={'caps:escape'}}},"
                       "['input-sources']={sources={{type='xkb',id='us'}},['per-window']=false},"
                       "['frame-renderers']={my_frame={'my_renderer'}},"
                       "shortcuts={{name='terminal',binding='<Super>Return',command={'new'}},"
                       "{name='keep',binding='<Super>k',command={'keep'}}},"
                       "autostart={{name='bar',command={'waybar'}}},"
                       "['window-rules']={{match={type='window'},opacity=1},"
                       "{match={['app-id']='my_app',focused=false},opacity=0.95,"
                       "['shader-uniforms']={my_strength=0.5},corners={['keep-maximized']=true}}},"
                       "permissions={rules={{name='capture',match='^app-id:my_app$',"
                       "capabilities={'screen-cast'},level='ask'}}}}\n");
    assert_equal(actual, expected);
    g_autoptr(GVariant) reloaded = evaluate(root, source);
    assert_equal(reloaded, expected);
    g_autoptr(GVariant) cleared = evaluate(
        root, "gnoblin.load('component.lua')\n"
              "gnoblin.remove_shortcut('terminal')\n"
              "assert(#gnoblin.config.shortcuts==1 and gnoblin.config.shortcuts[1].name=='keep')\n"
              "gnoblin.configure {shortcuts={},window_rules={}}\n"
              "assert(#gnoblin.config.shortcuts==0 and #gnoblin.config['window-rules']==0)\n"
              "gnoblin.configure {keybindings={shell={show_screenshot_ui={}}}}\n"
              "assert(#gnoblin.config.keybindings.shell.show_screenshot_ui==0)\n");
    g_autoptr(GVariant) named = evaluate(
        root, "gnoblin.load('component.lua')\n"
              "local snapshot=gnoblin.snapshot()\n"
              "assert(#snapshot.shortcuts==2)\n"
              "assert(gnoblin.configure.shortcuts.terminal.binding=='<Super>Return')\n"
              "local count=0; for name,entry in pairs(gnoblin.configure.shortcuts) do "
              "assert(name==entry.name); count=count+1 end; assert(count==2)\n"
              "snapshot.shortcuts[1].binding='changed'\n"
              "assert(gnoblin.config.shortcuts[1].binding=='<Super>Return')\n"
              "gnoblin.configure.shortcuts.keep.enable=false\n"
              "gnoblin.autostart {name='waybar',command={'waybar'}}\n"
              "gnoblin.configure.autostart.waybar.enable=false\n"
              "gnoblin.configure {shortcuts={terminal={command={'new'}},"
              "my_extra={binding='<Super>e',command={'extra'}}}}\n"
              "gnoblin.configure.shortcuts.my_extra.capture_input=true\n"
              "gnoblin.shortcut {name='my_extra',command={'updated'}}\n");
    g_autoptr(GVariant) named_expected = evaluate(
        root, "return {shortcuts={{name='terminal',binding='<Super>Return',command={'new'}},"
              "{name='my_extra',binding='<Super>e',command={'updated'},"
              "['capture-input']=true}},"
              "autostart={},"
              "['window-rules']={{match={type='window'},opacity=1}}}\n");
    assert_equal(named, named_expected);
    const char* invalid[] = {
        "gnoblin.window_rule(false)",
        "gnoblin.config=false; gnoblin.configure {shell={minimize_duration=150}}",
        "gnoblin.shortcut {command={'x'}}",
        "gnoblin.autostart {name=4}",
        "gnoblin.configure {shell={minimize_duration=1,['minimize-duration']=2}}",
        "gnoblin.config.shortcuts=false; gnoblin.shortcut {name='x'}",
        "gnoblin.config.shortcuts={bad={}}; gnoblin.remove_shortcut('x')",
        "gnoblin.shortcut {name='x',binding='<Super>x',command={'x'}}; "
        "gnoblin.configure.shortcuts.x.name='renamed'",
        "gnoblin.config.permissions=false; gnoblin.permission_rule {}",
        "local t={};t.child=t;gnoblin.configure(t)",
        NULL,
    };
    for (guint i = 0; invalid[i]; i++) {
        g_assert_true(g_file_set_contents(root, invalid[i], -1, &error));
        g_autoptr(GVariant) failed = gnoblin_config_load_document(root, NULL, NULL, &error);
        g_assert_null(failed);
        g_assert_nonnull(error);
        g_clear_error(&error);
    }
    g_unlink(root);
    g_unlink(component);
    g_rmdir(directory);
    g_print("PASS: declarations, named overrides, snake_case, imports and reloads\n");
    return 0;
}
