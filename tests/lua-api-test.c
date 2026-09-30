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
    const char* source =
        "assert(require('gnoblin') == gnoblin)\n"
        "gnoblin.load('component.lua')\n"
        "gnoblin.configure {layer_shell={preserve_active_window=true},"
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
        "gnoblin.permission_rule {name='capture',match='^app%-id:my_app$',"
        "capabilities={'screen-cast'},level='ask'}\n";
    g_autoptr(GVariant) actual = evaluate(root, source);
    g_autoptr(GVariant) expected = evaluate(
        root, "return {['layer-shell']={['preserve-active-window']=true},"
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
              "permissions={rules={{name='capture',match='^app%-id:my_app$',"
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
              "gnoblin.configure {shortcuts={screenshot={"
              "command={'grim'},binding={'Print'}}}}\n"
              "assert(#gnoblin.config.shortcuts==1 and "
              "gnoblin.config.shortcuts[1].command[1]=='grim')\n");
    g_autoptr(GVariant) named =
        evaluate(root, "gnoblin.load('component.lua')\n"
                       "local snapshot=gnoblin.snapshot()\n"
                       "assert(#snapshot.shortcuts==2)\n"
                       "assert(gnoblin.configure.shortcuts.terminal.binding=='<Super>Return')\n"
                       "local count=0; for name,entry in pairs(gnoblin.configure.shortcuts) do "
                       "assert(name==entry.name); count=count+1 end; assert(count==2)\n"
                       "snapshot.shortcuts[1].binding='changed'\n"
                       "assert(gnoblin.config.shortcuts[1].binding=='<Super>Return')\n"
                       "gnoblin.configure.shortcuts.keep.enable=false\n"
                       "gnoblin.autostart {name='waybar',command={'waybar'}}\n"
                       "assert(gnoblin.configure.autostart.waybar.enable)\n"
                       "gnoblin.configure.autostart.waybar.enable=false\n"
                       "gnoblin.configure {shortcuts={terminal={command={'new'}}}}\n"
                       "gnoblin.configure.shortcuts.my_extra="
                       "{binding='<Super>e',command={'extra'}}\n"
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
        "gnoblin.feature.list()",
        "gnoblin.feature.show {id='osd'}",
        "gnoblin.feature.enable {id='osd'}",
        "gnoblin.feature.disable {id='osd'}",
        "gnoblin.script.list()",
        "gnoblin.shell.ping()",
        "gnoblin.shell.version()",
        "gnoblin.shell.status()",
        "gnoblin.shell.reload()",
        "gnoblin.window_rule(false)",
        "gnoblin.config=false; gnoblin.configure {window_management={workspace_names={'x'}}}",
        "gnoblin.shortcut {command={'x'}}",
        "gnoblin.autostart {name=4}",
        "gnoblin.configure {window_management={constrain_drag_to_work_area=true,"
        "['constrain-drag-to-work-area']=false}}",
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

    const char* menu_runtime_source =
        "local g=require('gnoblin')\n"
        "local menu_calls=0\n"
        "g.on('gnoblin.window.menu-requested', function(event)\n"
        "  assert(event._menu_context_handle==nil and event._menu_context_expires_at_us==nil)\n"
        "  if event.menu_type=='wm' then\n"
        "    assert(type(event.menu_context)=='userdata')\n"
        "    local context=event.menu_context\n"
        "    menu_calls=menu_calls+1\n"
        "    if menu_calls==1 then context:begin_resize('south_east')\n"
        "    else context:begin_resize(42) end\n"
        "    assert(not pcall(function() context:begin_move() end))\n"
        "  else\n"
        "    assert(event.menu_context==nil)\n"
        "  end\n"
        "end)\n"
        "local thumbnail_events=0\n"
        "g.on('gnoblin.window.created', function()\n"
        "  thumbnail_events=thumbnail_events+1\n"
        "  if thumbnail_events==1 then\n"
        "    local operation=g.window.thumbnail({id='42',width=320,height=200})\n"
        "    assert(operation.method=='window.thumbnail' and operation.status=='pending')\n"
        "  else\n"
        "    assert(not pcall(function()\n"
        "      g.window.thumbnail({id='42',width=481,height=200})\n"
        "    end))\n"
        "  end\n"
        "end)\n";
    g_assert_true(g_file_set_contents(root, menu_runtime_source, -1, &error));
    g_autoptr(GVariant) menu_runtime = gnoblin_config_load_runtime(root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(menu_runtime);
    gnoblin_config_finish_load(TRUE);

    GVariantBuilder api_arguments_builder;
    g_variant_builder_init(&api_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) api_arguments =
        g_variant_ref_sink(g_variant_builder_end(&api_arguments_builder));
    g_autoptr(GVariant) api_operation =
        gnoblin_config_call_api("workspace.list", api_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(api_operation);
    g_autoptr(GVariant) api_method =
        g_variant_lookup_value(api_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(api_method);
    g_assert_cmpstr(g_variant_get_string(api_method, NULL), ==, "workspace.list");
    g_autoptr(GVariant) queued_api_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(queued_api_operations), ==, 1);
    g_autoptr(GVariant) queued_api_operation = g_variant_get_child_value(queued_api_operations, 0);
    g_assert_true(g_variant_equal(api_operation, queued_api_operation));

    GVariantBuilder thumbnail_event_builder;
    g_variant_builder_init(&thumbnail_event_builder, G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) thumbnail_event_payload =
        g_variant_ref_sink(g_variant_builder_end(&thumbnail_event_builder));
    g_autoptr(GVariant) thumbnail_event_result =
        gnoblin_config_dispatch_event("gnoblin.window.created", thumbnail_event_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(thumbnail_event_result);
    g_autoptr(GVariant) thumbnail_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(thumbnail_operations), ==, 1);
    g_autoptr(GVariant) thumbnail_operation = g_variant_get_child_value(thumbnail_operations, 0);
    g_autoptr(GVariant) thumbnail_method =
        g_variant_lookup_value(thumbnail_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(thumbnail_method, NULL), ==, "window.thumbnail");
    g_autoptr(GVariant) thumbnail_arguments =
        g_variant_lookup_value(thumbnail_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    const char* thumbnail_id = NULL;
    gint64 thumbnail_width = 0;
    gint64 thumbnail_height = 0;
    g_assert_true(g_variant_lookup(thumbnail_arguments, "id", "&s", &thumbnail_id));
    g_assert_true(g_variant_lookup(thumbnail_arguments, "width", "x", &thumbnail_width));
    g_assert_true(g_variant_lookup(thumbnail_arguments, "height", "x", &thumbnail_height));
    g_assert_cmpstr(thumbnail_id, ==, "42");
    g_assert_cmpint(thumbnail_width, ==, 320);
    g_assert_cmpint(thumbnail_height, ==, 200);
    gnoblin_config_finish_event(TRUE);

    g_autoptr(GVariant) invalid_thumbnail_result =
        gnoblin_config_dispatch_event("gnoblin.window.created", thumbnail_event_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(invalid_thumbnail_result);
    g_autoptr(GVariant) invalid_thumbnail_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(invalid_thumbnail_operations), ==, 0);
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder menu_event;
    g_variant_builder_init(&menu_event, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&menu_event, "{sv}", "window_id", g_variant_new_string("42"));
    g_variant_builder_add(&menu_event, "{sv}", "menu_type", g_variant_new_string("wm"));
    g_variant_builder_add(&menu_event, "{sv}", "x", g_variant_new_int32(300));
    g_variant_builder_add(&menu_event, "{sv}", "y", g_variant_new_int32(200));
    g_variant_builder_add(&menu_event, "{sv}", "_menu_context_handle", g_variant_new_uint64(77));
    g_variant_builder_add(&menu_event, "{sv}", "_menu_context_generation", g_variant_new_uint64(9));
    g_variant_builder_add(&menu_event, "{sv}", "_menu_context_expires_at_us",
                          g_variant_new_int64(g_get_monotonic_time() + G_USEC_PER_SEC));
    g_autoptr(GVariant) menu_payload = g_variant_ref_sink(g_variant_builder_end(&menu_event));
    g_autoptr(GVariant) menu_result =
        gnoblin_config_dispatch_event("gnoblin.window.menu-requested", menu_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(menu_result);
    g_autoptr(GVariant) menu_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(menu_operations), ==, 1);
    g_autoptr(GVariant) menu_operation = g_variant_get_child_value(menu_operations, 0);
    g_autoptr(GVariant) menu_method =
        g_variant_lookup_value(menu_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(menu_method, NULL), ==, "window.begin_resize");
    g_autoptr(GVariant) menu_arguments =
        g_variant_lookup_value(menu_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    const char* menu_edge = NULL;
    g_assert_true(g_variant_lookup(menu_arguments, "edge", "&s", &menu_edge));
    g_assert_cmpstr(menu_edge, ==, "south_east");
    g_autoptr(GVariant) menu_handle =
        g_variant_lookup_value(menu_arguments, "_menu_context_handle", G_VARIANT_TYPE_UINT64);
    g_assert_nonnull(menu_handle);
    g_assert_null(g_variant_lookup_value(menu_arguments, "id", NULL));
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder malformed_menu;
    g_variant_builder_init(&malformed_menu, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&malformed_menu, "{sv}", "window_id", g_variant_new_string("42"));
    g_variant_builder_add(&malformed_menu, "{sv}", "menu_type", g_variant_new_string("wm"));
    g_variant_builder_add(&malformed_menu, "{sv}", "_menu_context_handle",
                          g_variant_new_uint64(78));
    g_variant_builder_add(&malformed_menu, "{sv}", "_menu_context_generation",
                          g_variant_new_uint64(9));
    g_variant_builder_add(&malformed_menu, "{sv}", "_menu_context_expires_at_us",
                          g_variant_new_int64(g_get_monotonic_time() + G_USEC_PER_SEC));
    g_autoptr(GVariant) malformed_menu_payload =
        g_variant_ref_sink(g_variant_builder_end(&malformed_menu));
    g_autoptr(GVariant) malformed_menu_result = gnoblin_config_dispatch_event(
        "gnoblin.window.menu-requested", malformed_menu_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(malformed_menu_result);
    g_autoptr(GVariant) malformed_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(malformed_operations), ==, 1);
    g_autoptr(GVariant) malformed_operation = g_variant_get_child_value(malformed_operations, 0);
    g_autoptr(GVariant) malformed_arguments =
        g_variant_lookup_value(malformed_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    g_autoptr(GVariant) malformed_marker =
        g_variant_lookup_value(malformed_arguments, "_malformed", G_VARIANT_TYPE_BOOLEAN);
    g_assert_nonnull(malformed_marker);
    g_assert_true(g_variant_get_boolean(malformed_marker));
    gnoblin_config_finish_event(TRUE);

    GVariantBuilder app_menu;
    g_variant_builder_init(&app_menu, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&app_menu, "{sv}", "window_id", g_variant_new_string("42"));
    g_variant_builder_add(&app_menu, "{sv}", "menu_type", g_variant_new_string("app"));
    g_autoptr(GVariant) app_menu_payload = g_variant_ref_sink(g_variant_builder_end(&app_menu));
    g_autoptr(GVariant) app_menu_result =
        gnoblin_config_dispatch_event("gnoblin.window.menu-requested", app_menu_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(app_menu_result);
    g_autoptr(GVariant) app_menu_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(app_menu_operations), ==, 0);
    gnoblin_config_finish_event(TRUE);

    g_unlink(root);
    g_unlink(component);
    g_rmdir(directory);
    g_print("PASS: declarations, named overrides, snake_case, imports and reloads\n");
    return 0;
}
