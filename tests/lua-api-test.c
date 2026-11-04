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

/* An output record as the compositor publishes it, with a privacy screen when `privacy`. */
static GVariant* test_output(const char* id, gboolean primary, double scale, gboolean privacy) {
    GVariantBuilder mode;
    g_variant_builder_init(&mode, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&mode, "{sv}", "width", g_variant_new_int32(1280));
    g_variant_builder_add(&mode, "{sv}", "height", g_variant_new_int32(800));
    g_variant_builder_add(&mode, "{sv}", "refresh_rate", g_variant_new_double(60.0));
    GVariantBuilder work_area;
    g_variant_builder_init(&work_area, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&work_area, "{sv}", "x", g_variant_new_int32(0));
    g_variant_builder_add(&work_area, "{sv}", "y", g_variant_new_int32(0));
    g_variant_builder_add(&work_area, "{sv}", "width", g_variant_new_int32(1280));
    g_variant_builder_add(&work_area, "{sv}", "height", g_variant_new_int32(768));
    GVariantBuilder privacy_screen;
    g_variant_builder_init(&privacy_screen, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&privacy_screen, "{sv}", "available", g_variant_new_boolean(privacy));
    if (privacy) {
        g_variant_builder_add(&privacy_screen, "{sv}", "enabled", g_variant_new_boolean(FALSE));
        g_variant_builder_add(&privacy_screen, "{sv}", "locked", g_variant_new_boolean(FALSE));
    }
    g_variant_builder_add(&privacy_screen, "{sv}", "requested", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&privacy_screen, "{sv}", "source", g_variant_new_string("system"));
    GVariantBuilder output;
    g_variant_builder_init(&output, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&output, "{sv}", "id", g_variant_new_string(id));
    g_variant_builder_add(&output, "{sv}", "kind", g_variant_new_string("display"));
    g_variant_builder_add(&output, "{sv}", "enabled", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&output, "{sv}", "primary", g_variant_new_boolean(primary));
    g_variant_builder_add(&output, "{sv}", "scale", g_variant_new_double(scale));
    g_variant_builder_add(&output, "{sv}", "mode", g_variant_builder_end(&mode));
    g_variant_builder_add(&output, "{sv}", "work_area", g_variant_builder_end(&work_area));
    g_variant_builder_add(&output, "{sv}", "privacy_screen",
                          g_variant_builder_end(&privacy_screen));
    return g_variant_builder_end(&output);
}

static gsize dispatch_counting_operations(const char* event, GVariant* payload);

/* Run the callbacks that finished Operations queued, and commit what they changed. */
static void run_deferred_callbacks(void) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) document = gnoblin_config_dispatch_deferred_callbacks(&error);
    g_assert_no_error(error);
    if (document)
        gnoblin_config_finish_event(TRUE);
    g_autoptr(GVariant) operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(operations), ==, 0);
}

/* The payload of a test step: {step = name}. */
static GVariant* step_payload(const char* step) {
    GVariantBuilder builder;
    g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&builder, "{sv}", "step", g_variant_new_string(step));
    return g_variant_builder_end(&builder);
}

/* Dispatch `event` with `payload` (sunk here) and return the request IDs of the operations it
 * queued, in order. */
static GArray* dispatch_operation_ids(const char* event, GVariant* payload) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) sunk = g_variant_ref_sink(payload);
    g_autoptr(GVariant) result = gnoblin_config_dispatch_event(event, sunk, &error);
    g_assert_no_error(error);
    g_assert_nonnull(result);
    g_autoptr(GVariant) operations = gnoblin_config_drain_runtime_operations();
    gnoblin_config_finish_event(TRUE);
    GArray* ids = g_array_new(FALSE, FALSE, sizeof(gint64));
    for (gsize i = 0; i < g_variant_n_children(operations); i++) {
        g_autoptr(GVariant) operation = g_variant_get_child_value(operations, i);
        gint64 id = 0;
        g_assert_true(g_variant_lookup(operation, "request_id", "x", &id));
        g_array_append_val(ids, id);
    }
    return ids;
}

/* Complete operation `id` as the compositor does: `value` (sunk here) on success, or an error
 * with `message`. */
static void complete_operation(gint64 id, gboolean ok, GVariant* value, const char* message) {
    GVariantBuilder completion;
    g_variant_builder_init(&completion, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&completion, "{sv}", "operation_id", g_variant_new_int64(id));
    g_variant_builder_add(&completion, "{sv}", "ok", g_variant_new_boolean(ok));
    if (value)
        g_variant_builder_add(&completion, "{sv}", "value", value);
    if (message) {
        GVariantBuilder failure;
        g_variant_builder_init(&failure, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&failure, "{sv}", "code", g_variant_new_string("failed"));
        g_variant_builder_add(&failure, "{sv}", "message", g_variant_new_string(message));
        g_variant_builder_add(&completion, "{sv}", "error", g_variant_builder_end(&failure));
    }
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.operation.completed",
                                                  g_variant_builder_end(&completion)),
                     ==, 0);
}

/* Dispatch `event` with `payload` (sunk here) and return how many operations it queued. */
static gsize dispatch_counting_operations(const char* event, GVariant* payload) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GVariant) sunk = g_variant_ref_sink(payload);
    g_autoptr(GVariant) result = gnoblin_config_dispatch_event(event, sunk, &error);
    g_assert_no_error(error);
    g_assert_nonnull(result);
    g_autoptr(GVariant) operations = gnoblin_config_drain_runtime_operations();
    gnoblin_config_finish_event(TRUE);
    return g_variant_n_children(operations);
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
                         "gnoblin.configure {layer_shell={preserve_active_window=true},"
                         "window_management={constrain_drag_to_work_area=true},"
                         "workspaces={{id='main',name='Main'},{id='chat',name='Chat'}},"
                         "compositor={visual_bell=true},"
                         "input={orientation_lock=true,keyboard={xkb_options={'caps:escape'}}},"
                         "input_sources={sources={{type='xkb',id='us'}},per_window=false},"
                         "frame_renderers={my_frame={'my_renderer'}}}\n"
                         "gnoblin.configure {shortcuts={terminal={command={'new'}},"
                         "temporary={command={'unused'}}}}\n"
                         "gnoblin.configure.shortcuts.temporary.enable=false\n"
                         "gnoblin.configure {autostart={bar={command={'old'}}}}\n"
                         "gnoblin.configure {autostart={bar={command={'waybar'}},"
                         "remove={command={'unused'}}}}\n"
                         "gnoblin.configure.autostart.remove.enable=false\n"
                         "local rule={match={app_id='my_app',focused=false},opacity=0.95,"
                         "shader_uniforms={my_strength=0.5},corners={keep_maximized=true}}\n"
                         "gnoblin.window_rule(rule)\n"
                         "rule.opacity=0.1\n"
                         "assert(rule.match.app_id == 'my_app')\n";
    g_autoptr(GVariant) actual = evaluate(root, source);
    g_autoptr(GVariant) expected = evaluate(
        root, "return {['layer-shell']={['preserve-active-window']=true},"
              "['window-management']={['constrain-drag-to-work-area']=true},"
              "workspaces={{id='main',name='Main'},{id='chat',name='Chat'}},"
              "compositor={['visual-bell']=true},"
              "input={['orientation-lock']=true,keyboard={['xkb-options']={'caps:escape'}}},"
              "['input-sources']={sources={{type='xkb',id='us'}},['per-window']=false},"
              "['frame-renderers']={my_frame={'my_renderer'}},"
              "shortcuts={{name='terminal',binding='<Super>Return',command={'new'}},"
              "{name='keep',binding='<Super>k',command={'keep'}}},"
              "autostart={{name='bar',command={'waybar'}}},"
              "['window-rules']={{match={type='window'},opacity=1},"
              "{match={['app-id']='my_app',focused=false},opacity=0.95,"
              "['shader-uniforms']={my_strength=0.5},corners={['keep-maximized']=true}}}}\n");
    assert_equal(actual, expected);
    g_autoptr(GVariant) reloaded = evaluate(root, source);
    assert_equal(reloaded, expected);
    g_autoptr(GVariant) cleared =
        evaluate(root, "gnoblin.load('component.lua')\n"
                       "gnoblin.configure.shortcuts.terminal.enable=false\n"
                       "gnoblin.configure {shortcuts={},window_rules={}}\n"
                       "assert(#gnoblin.snapshot().shortcuts==0 and "
                       "#gnoblin.snapshot()['window-rules']==0)\n"
                       "gnoblin.configure {shortcuts={screenshot={"
                       "command={'grim'},binding={'Print'}}}}\n"
                       "assert(#gnoblin.snapshot().shortcuts==1 and "
                       "gnoblin.snapshot().shortcuts[1].command[1]=='grim')\n");
    g_autoptr(GVariant) named =
        evaluate(root, "gnoblin.load('component.lua')\n"
                       "local snapshot=gnoblin.snapshot()\n"
                       "assert(#snapshot.shortcuts==2)\n"
                       "assert(gnoblin.configure.shortcuts.terminal.binding=='<Super>Return')\n"
                       "local count=0; for name,entry in pairs(gnoblin.configure.shortcuts) do "
                       "assert(name==entry.name); count=count+1 end; assert(count==2)\n"
                       "snapshot.shortcuts[1].binding='changed'\n"
                       "assert(gnoblin.snapshot().shortcuts[1].binding=='<Super>Return')\n"
                       "gnoblin.configure.shortcuts.keep.enable=false\n"
                       "gnoblin.configure.autostart.waybar={command={'waybar'}}\n"
                       "assert(gnoblin.configure.autostart.waybar.command[1]=='waybar')\n"
                       "gnoblin.configure.autostart.waybar.enable=false\n"
                       "gnoblin.configure {shortcuts={terminal={command={'new'}}}}\n"
                       "gnoblin.configure.shortcuts.my_extra="
                       "{binding='<Super>e',command={'extra'}}\n"
                       "gnoblin.configure.shortcuts.my_extra.capture_input=true\n"
                       "gnoblin.configure.shortcuts.my_extra.command={'updated'}\n");
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
        "gnoblin.set {window_management={focus_mode='click'}}",
        "gnoblin.window_rule(false)",
        "gnoblin.configure {window_management={workspace_names={'x'}}}",
        "gnoblin.configure {window_management={workspace_ids={'main'}}}",
        "gnoblin.configure {window_management={num_workspaces=2}}",
        "gnoblin.configure {window_management={dynamic_workspaces=true},"
        "workspaces={{id='main',name='Main'}}}",
        "gnoblin.shortcut {command={'x'}}",
        "gnoblin.autostart {name=4}",
        "gnoblin.configure {window_management={constrain_drag_to_work_area=true,"
        "['constrain-drag-to-work-area']=false}}",
        "gnoblin.config.shortcuts=false",
        "gnoblin.config.compositor={debug=true}",
        "gnoblin.config.anything=1",
        "local ignored=gnoblin.config.shortcuts",
        "gnoblin.shortcut {name='x',binding='<Super>x',command={'x'}}; "
        "gnoblin.configure.shortcuts.x.name='renamed'",
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
    const char* removed_api_calls[] = {
        "gnoblin.shortcut {name='terminal',command={'foot'}}",
        "gnoblin.autostart {name='bar',command={'waybar'}}",
        "gnoblin.remove_shortcut('terminal')",
        "gnoblin.remove_autostart('bar')",
        NULL,
    };
    const char* migration_hints[] = {
        "gnoblin.configure.shortcuts.name",
        "gnoblin.configure.autostart.name",
        "gnoblin.configure.shortcuts.name.enable = false",
        "gnoblin.configure.autostart.name.enable = false",
    };
    for (guint i = 0; removed_api_calls[i]; i++) {
        g_assert_true(g_file_set_contents(root, removed_api_calls[i], -1, &error));
        g_autoptr(GVariant) failed = gnoblin_config_load_document(root, NULL, NULL, &error);
        g_assert_null(failed);
        g_assert_nonnull(error);
        g_assert_nonnull(strstr(error->message, migration_hints[i]));
        g_clear_error(&error);
    }

    const char* menu_runtime_source =
        "local g=require('gnoblin')\n"
        "assert(g.input.select==nil and g.input.select_source==nil)\n"
        "local menu_calls=0\n"
        "g.on('gnoblin.window.menu-requested', function(event)\n"
        "  assert(event._menu_context_handle==nil and event._menu_context_expires_at_us==nil)\n"
        "  assert(event.menu_context==nil)\n"
        "  if event.menu_type=='wm' then\n"
        "    assert(type(event.grant)=='userdata')\n"
        "    local grant=event.grant\n"
        "    local window=g.windows.by_id(event.window_id)\n"
        "    assert(#grant.scope==2 and grant.window.id=='42' and grant.alive)\n"
        "    assert(not pcall(function() grant:narrow('focus') end))\n"
        "    assert(not pcall(function() grant:narrow('begin_move', 'other') end))\n"
        "    assert(not pcall(function() window:focus(grant) end))\n"
        "    menu_calls=menu_calls+1\n"
        "    if menu_calls==1 then window:begin_resize('south_east', grant)\n"
        "    else\n"
        "      assert(not pcall(function() window:begin_resize(42, grant) end))\n"
        "      assert(not pcall(function() window:begin_resize('sideways', grant) end))\n"
        "      window:begin_move(grant)\n"
        "    end\n"
        "    assert(not pcall(function() window:begin_move(grant) end))\n"
        "    assert(not grant.alive and #grant.scope==0)\n"
        "  else\n"
        "    assert(event.grant==nil and event.menu_context==nil)\n"
        "  end\n"
        "end)\n"
        "assert(g.window == nil)\n"
        "local thumbnail_events=0\n"
        "g.on('gnoblin.window.created', function(event)\n"
        "  thumbnail_events=thumbnail_events+1\n"
        "  if thumbnail_events==1 then\n"
        "    local operation=event.window:thumbnail(320,200)\n"
        "    assert(operation.method=='window.thumbnail' and operation.status=='pending')\n"
        "  else\n"
        "    assert(not pcall(function()\n"
        "      event.window:thumbnail(481,200)\n"
        "    end))\n"
        "  end\n"
        "end)\n"
        "local output, output_log=nil, {}\n"
        "g.on('test.outputs', function(event)\n"
        "  if event.step=='read' then\n"
        "    assert(g.monitors==nil)\n"
        "    local list=g.outputs.list()\n"
        "    output=g.outputs.by_id('eDP-1')\n"
        "    assert(#list==2 and list[1]==output and g.outputs.primary()==output and "
        "output.alive)\n"
        "    assert(output.privacy_screen.available and output.work_area.width==1280)\n"
        "    output:watch('scale', function(o, value, old)\n"
        "      output_log[#output_log+1]=string.format('%g>%g', old, value)\n"
        "    end)\n"
        "    output:on('changed', function(o, changed)\n"
        "      output_log[#output_log+1]=table.concat(changed, ',')\n"
        "    end)\n"
        "    output:on('closed', function(o) output_log[#output_log+1]='closed' end)\n"
        "    assert(not pcall(function() output:watch('missing', print) end))\n"
        "    assert(output.set_privacy_screen==nil and g.outputs.set_privacy_screen==nil)\n"
        "    assert(g.input.set_orientation_lock==nil)\n"
        "    assert(not pcall(function() output:set_privacy_screen(true) end))\n"
        "    assert(not pcall(g.settings.override, 'monitors.privacy_screen', 'on'))\n"
        "    assert(not pcall(g.settings.override, 'monitors.privacy_screen', 'inherit'))\n"
        "    assert(not pcall(g.settings.override, 'monitors.privacy_screen'))\n"
        "    assert(not pcall(g.settings.override, 'monitors.privacy_screen', true, false))\n"
        "    local override=g.settings.override('monitors.privacy_screen', true)\n"
        "    assert(override.alive and override.scope=='settings' and override.value==true)\n"
        "  else\n"
        "    assert(table.concat(output_log, ';')=='1>2;scale;closed', table.concat(output_log, "
        "';'))\n"
        "    assert(not output.alive and output.scale==2)\n"
        "    assert(not pcall(function() output:on('changed', print) end))\n"
        "    g.workspaces.next()\n"
        "  end\n"
        "end)\n"
        "g.on('test.no-privacy', function(event)\n"
        "  local ok, err=pcall(g.settings.override, 'monitors.privacy_screen', false)\n"
        "  assert(not ok and tostring(err):find('no output has a privacy screen', 1, true), "
        "tostring(err))\n"
        "  local operation=g.settings.override('compositor.visual_bell', false)\n"
        "  assert(operation.alive)\n"
        "end)\n"
        "g.on('test.find', function(event)\n"
        "  local function refused(fragment, ...)\n"
        "    local ok, err=pcall(g.find, ...)\n"
        "    assert(not ok and tostring(err):find(fragment, 1, true), tostring(err))\n"
        "  end\n"
        "  assert(#g.find('Window')==#g.windows.list() and #g.find('Output')==#g.outputs.list())\n"
        "  local window=g.windows.by_id('42')\n"
        "  assert(g.find('Window', {id='42'})[1]==window and #g.find('Window', {id='42'})==1)\n"
        "  assert(#g.find('Window', {id='42', alive=true})==1)\n"
        "  assert(#g.find('Window', {id='none'})==0 and #g.find('Window', {title='x'})==0)\n"
        "  assert(#g.find('Window', function(w) return w.frame.width==800 end)==1)\n"
        "  assert(#g.find('Window', function(w) return false end)==0)\n"
        "  assert(g.find('Output', {primary=true})[1]==g.outputs.primary())\n"
        "  assert(#g.find('Output', {id='HDMI-1'})==1 and #g.find('Output', {id='eDP-1'})==0)\n"
        "  assert(#g.find('Process')==0 and #g.find('Shortcut')==#g.shortcuts.list())\n"
        "  refused('has no nonsense field', 'Window', {nonsense=1})\n"
        "  refused('Window.frame is not a scalar field', 'Window', {frame=1})\n"
        "  refused('cannot list Tween', 'Tween')\n"
        "  refused('cannot list banana', 'banana')\n"
        "  refused('filter must be a table of fields or a function', 'Window', 5)\n"
        "  refused('compares Window.id with a string, number or boolean', 'Window', {id={}})\n"
        "  refused('filter keys are field names', 'Window', {'x'})\n"
        "  refused('takes a kind and an optional filter', 'Window', nil, 1)\n"
        "  assert(not pcall(g.windows.list, {id='42'}) and not pcall(g.layers.list, {}))\n"
        "  assert(g.animations.surfaces==nil and g.animations.inspect==nil)\n"
        "  assert(type(window.inspect_animation)=='function' and "
        "type(window.preview_animation)=='function')\n"
        "  assert(not pcall(window.inspect_animation, window))\n"
        "  assert(not pcall(window.preview_animation, window, 'a', 5))\n"
        "  assert(not pcall(window.move_to_workspace, window, {}))\n"
        "  assert(not pcall(window.move_to_monitor, window, 5))\n"
        "  g.workspaces.next()\n"
        "end)\n"
        "local configure_log={}\n"
        "g.on('test.configure', function(event)\n"
        "  if event.step=='start' then\n"
        "    assert(not pcall(function() g.config.compositor={} end))\n"
        "    assert(not pcall(function() return g.config.compositor end))\n"
        "    assert(not pcall(g.snapshot))\n"
        "    assert(type(g.config.path)=='function')\n"
        "    assert(not pcall(function()\n"
        "      g.configure.shortcuts.x={binding='<Super>x', command={'x'}}\n"
        "    end))\n"
        "    local operation=g.configure {cursor={size=31}}\n"
        "    assert(operation.status=='pending' and operation.method=='configure')\n"
        "    operation:on_complete(function(value, err)\n"
        "      configure_log[#configure_log+1]=err and err.code or 'succeeded'\n"
        "    end)\n"
        "    local rule=g.window_rule {match={type='window'}, opacity=0.5}\n"
        "    assert(rule.status=='pending')\n"
        "    rule:on_complete(function(value, err)\n"
        "      configure_log[#configure_log+1]=err and err.code or 'succeeded'\n"
        "    end)\n"
        "  elseif event.step=='reject' then\n"
        "    local operation=g.configure {cursor={size='big'}}\n"
        "    operation:on_complete(function(value, err)\n"
        "      configure_log[#configure_log+1]='rejected:'..tostring(err and err.code)\n"
        "    end)\n"
        "  else\n"
        "    assert(table.concat(configure_log, ';')=='succeeded;succeeded;"
        "rejected:invalid_argument', table.concat(configure_log, ';'))\n"
        "    assert(g.settings.cursor.size==31)\n"
        "    g.workspaces.next()\n"
        "  end\n"
        "end)\n"
        "local launch, launch_log, replaced_log=nil, {}, {}\n"
        "g.on('test.launches', function(event)\n"
        "  if event.step=='begin' then\n"
        "    assert(g.launches.finish==nil)\n"
        "    launch=g.launches.begin('tok', 'org.example.Editor', {timeout_ms=500})\n"
        "    assert(launch.alive and launch.state=='pending' and launch.token=='tok')\n"
        "    assert(launch.timeout_ms==500 and launch.revision==0)\n"
        "    launch:on('changed', function(l, changed)\n"
        "      table.sort(changed)\n"
        "      launch_log[#launch_log+1]='changed:'..table.concat(changed, ',')\n"
        "    end)\n"
        "    launch:on('started', function(l) launch_log[#launch_log+1]='started' end)\n"
        "    launch:on('timed_out', function(l) launch_log[#launch_log+1]='timed_out' end)\n"
        "    launch:on('closed', function(l, reason)\n"
        "      launch_log[#launch_log+1]='closed:'..tostring(reason)\n"
        "    end)\n"
        "    launch:watch('state', function(l, value, old)\n"
        "      launch_log[#launch_log+1]=old..'>'..value\n"
        "    end)\n"
        "    assert(not pcall(function() launch:on('bogus', print) end))\n"
        "    assert(not pcall(function() launch:watch('bogus', print) end))\n"
        "    local first=g.launches.begin('rep', 'x.y')\n"
        "    first:on('closed', function(l, reason) replaced_log[#replaced_log+1]=reason end)\n"
        "    local again=g.launches.begin('rep', 'x.y')\n"
        "    assert(first~=again and not first.alive and again.alive and "
        "replaced_log[1]=='replaced')\n"
        "    local finish=launch:finish()\n"
        "    assert(finish.method=='launch.end')\n"
        "    assert(not pcall(function() launch:finish(true) end))\n"
        "  else\n"
        "    assert(table.concat(launch_log, ';')=="
        "'pending>started;changed:started_at,state;started;closed:nil', "
        "table.concat(launch_log, ';'))\n"
        "    assert(not launch.alive and launch.state=='started' and launch.revision==2)\n"
        "    assert(not pcall(function() launch:on('changed', print) end))\n"
        "    assert(launch.state=='started')\n"
        "    local reused=g.launches.begin('tok', 'org.example.Editor')\n"
        "    assert(reused~=launch and reused.alive and not launch.alive)\n"
        "  end\n"
        "end)\n"
        "local shortcut, shortcut_log=nil, {}\n"
        "g.on('test.shortcuts', function(event)\n"
        "  if event.step=='bind' then\n"
        "    assert(g.shortcuts.unbind==nil and g.shortcuts.end_session==nil)\n"
        "    shortcut=g.shortcuts.bind('sw', '<Super>space', {hold='super', mode='modal'})\n"
        "    assert(shortcut.alive and shortcut.hold=='super' and shortcut.mode=='modal')\n"
        "    assert(shortcut.trigger=='press' and not shortcut.capture_input)\n"
        "    assert(#g.shortcuts.list()==1 and g.shortcuts.list()[1]==shortcut)\n"
        "    assert(not pcall(function() g.shortcuts.bind('sw', '<Super>x') end))\n"
        "    assert(shortcut.accelerator=='<Super>space')\n"
        "    local ok, err=pcall(g.shortcuts.bind, 'old', 'Super+Alt+F9')\n"
        "    assert(not ok and tostring(err):find('Write <Alt><Super>F9', 1, true), "
        "tostring(err))\n"
        "    ok, err=pcall(g.shortcuts.bind, 'old', '<Windows>q')\n"
        "    assert(not ok and tostring(err):find('<Windows> is not a modifier', 1, true))\n"
        "    assert(#g.shortcuts.list()==1)\n"
        "    assert(not pcall(function() shortcut:on('bogus', print) end))\n"
        "    assert(not pcall(function() shortcut:end_session(0) end))\n"
        "    shortcut:on('activated', function(s, e)\n"
        "      shortcut_log[#shortcut_log+1]='activated:'..e.id..':'..tostring(e.first)\n"
        "    end)\n"
        "    shortcut:on('session_activated', function(s, e)\n"
        "      shortcut_log[#shortcut_log+1]='session:'..e.session_id\n"
        "    end)\n"
        "    shortcut:on('session_ended', function(s, e)\n"
        "      shortcut_log[#shortcut_log+1]='ended:'..e.reason\n"
        "    end)\n"
        "    shortcut:on('closed', function(s, reason)\n"
        "      shortcut_log[#shortcut_log+1]='closed:'..tostring(reason)\n"
        "    end)\n"
        "    assert(shortcut:end_session(3).method=='shortcut.session.end')\n"
        "    assert(shortcut:unbind().method=='shortcut.unbind')\n"
        "    local refused=g.shortcuts.bind('refused', '<Super>y')\n"
        "    refused:on('closed', function(s, reason)\n"
        "      shortcut_log[#shortcut_log+1]='refused:'..tostring(reason)\n"
        "    end)\n"
        "  else\n"
        "    assert(table.concat(shortcut_log, ';')=="
        "'refused:taken;activated:sw:true;session:3;ended:cancelled;closed:nil', "
        "table.concat(shortcut_log, ';'))\n"
        "    assert(not shortcut.alive and #g.shortcuts.list()==0)\n"
        "    assert(not pcall(function() shortcut:on('closed', print) end))\n"
        "    local rebound=g.shortcuts.bind('sw', '<Super>space')\n"
        "    assert(rebound~=shortcut and rebound.alive and #g.shortcuts.list()==1)\n"
        "  end\n"
        "end)\n"
        "local idle_watch, idle_log, second_watch=nil, {}, nil\n"
        "local function note(text) idle_log[#idle_log+1]=text end\n"
        "g.on('test.idle', function(event)\n"
        "  if event.step=='watch' then\n"
        "    assert(g.session.activity==nil)\n"
        "    for _, bad in ipairs({0, -1, 86401, 'x', 0/0}) do\n"
        "      assert(not pcall(function() g.idle.watch(bad) end))\n"
        "    end\n"
        "    assert(not pcall(function() g.idle.watch() end))\n"
        "    assert(not pcall(function() g.idle.watch(1, 2) end))\n"
        "    idle_watch=g.idle.watch(90)\n"
        "    assert(idle_watch.alive and not idle_watch.idle and idle_watch.seconds==90)\n"
        "    assert(idle_watch.id:match('^idle%-%d+$'))\n"
        "    second_watch=g.idle.watch(0.5)\n"
        "    assert(second_watch~=idle_watch and second_watch.seconds==0.5)\n"
        "    assert(not pcall(function() idle_watch:on('bogus', print) end))\n"
        "    idle_watch:on('idle', function(w) note('idle:'..tostring(w.idle)) end)\n"
        "    idle_watch:on('active', function(w) note('active:'..tostring(w.idle)) end)\n"
        "    idle_watch:watch('idle', function(w, value, old)\n"
        "      note('watch:'..tostring(old)..'>'..tostring(value))\n"
        "    end)\n"
        "    idle_watch:on('closed', function(w, reason) note('closed:'..tostring(reason)) end)\n"
        "    local refused=g.idle.watch(5)\n"
        "    refused:on('closed', function(w, reason)\n"
        "      note('refused:'..tostring(reason)..':'..tostring(w.alive))\n"
        "    end)\n"
        "  elseif event.step=='cancel' then\n"
        "    assert(idle_watch:cancel().method=='idle.cancel')\n"
        "    assert(idle_watch.alive)\n"
        "  else\n"
        "    assert(table.concat(idle_log, ';')=="
        "'refused:full:false;watch:false>true;idle:true;watch:true>false;active:false;closed:nil', "
        "table.concat(idle_log, ';'))\n"
        "    assert(not idle_watch.alive and second_watch.alive)\n"
        "    assert(not pcall(function() idle_watch:on('idle', print) end))\n"
        "  end\n"
        "end)\n"
        "local ov_log, ov = {}, {}\n"
        "local function ovnote(text) ov_log[#ov_log+1]=text end\n"
        "local function refusal(path, value, needle)\n"
        "  local ok, err=pcall(g.settings.override, path, value)\n"
        "  assert(not ok and tostring(err):find(needle, 1, true), path..': '..tostring(err))\n"
        "end\n"
        "g.on('test.overrides', function(event)\n"
        "  if event.step=='make' then\n"
        "    refusal('xwayland.enable', true, 'Xwayland')\n"
        "    refusal('input.mouse', true, 'input.orientation_lock')\n"
        "    refusal('window_rules.x', true, 'Window:set_effect')\n"
        "    refusal('shortcuts.x', true, 'gnoblin.shortcuts.bind')\n"
        "    refusal('workspaces.main', true, 'gnoblin.workspaces.create')\n"
        "    refusal('cursor.hide', true, 'cursor.theme and cursor.size')\n"
        "    refusal('nonsense.key', true, 'read only when the configuration loads')\n"
        "    refusal('window_management.nope', true, 'window-management')\n"
        "    refusal('compositor.blur', 'sideways', 'compositor')\n"
        "    refusal('window_management.auto_raise_delay', 99999, 'window-management')\n"
        "    refusal('input.orientation_lock', 'inherit', 'true or false')\n"
        "    refusal('compositor', true, 'such as window_management.focus_mode')\n"
        "    refusal('a.b.c', true, 'such as window_management.focus_mode')\n"
        "    assert(not pcall(g.settings.override, 'compositor.visual_bell'))\n"
        "    assert(not pcall(g.settings.override, 'compositor.visual_bell', true, false))\n"
        "    assert(not pcall(g.settings.override, 'compositor.visual_bell', {}))\n"
        "    for key in pairs(g.settings) do assert(key~='override') end\n"
        "    ov.bell=g.settings.override('compositor.visual_bell', false)\n"
        "    assert(ov.bell.alive and ov.bell.scope=='settings' and ov.bell.value==false)\n"
        "    assert(ov.bell.name=='compositor.visual_bell' and ov.bell.window==nil)\n"
        "    assert(ov.bell.id:match('^override%-%d+$'))\n"
        "    assert(not pcall(function() ov.bell:on('bogus', print) end))\n"
        "    ov.bell:on('closed', function(o, reason) ovnote('bell:'..tostring(reason)) end)\n"
        "    ov.lock=g.settings.override('input.orientation_lock', true)\n"
        "    ov.lock:on('closed', function(o, reason) ovnote('lock:'..tostring(reason)) end)\n"
        "    ov.refused=g.settings.override('window_management.focus_mode', 'hover')\n"
        "    ov.refused:on('closed', function(o, reason)\n"
        "      ovnote('refused:'..tostring(reason)..':'..tostring(o.alive))\n"
        "    end)\n"
        "    local window=event.window\n"
        "    assert(not pcall(function() window:set_effect('bogus', 1) end))\n"
        "    local ok, err=pcall(function() window:set_effect('bogus', 1) end)\n"
        "    assert(tostring(err):find('blur_ignore_shadows', 1, true), tostring(err))\n"
        "    assert(not pcall(function() window:set_effect('opacity', 2) end))\n"
        "    assert(not pcall(function() window:set_effect('blur', -1) end))\n"
        "    assert(not pcall(function() window:set_effect('blur_ignore_shadows', 'x') end))\n"
        "    assert(not pcall(function() window:set_effect('corners', {bogus=1}) end))\n"
        "    assert(not pcall(function() window:set_effect('corners', {radius=201}) end))\n"
        "    assert(not pcall(function() window:set_effect('shader_uniforms', {a='x'}) end))\n"
        "    assert(not pcall(function() window:set_effect('blur') end))\n"
        "    ov.blur=window:set_effect('blur', 20)\n"
        "    assert(ov.blur.alive and ov.blur.scope=='window' and ov.blur.window=='42')\n"
        "    assert(ov.blur.name=='blur' and ov.blur.value==20)\n"
        "    ov.blur:on('closed', function(o, reason) ovnote('blur:'..tostring(reason)) end)\n"
        "    ov.corners=window:set_effect('corners', {radius=12, border_width=2})\n"
        "    assert(ov.corners.value.radius==12 and ov.corners.value.border_width==2)\n"
        "    ov.corners:on('closed', function(o, reason) error('MARK '..tostring(reason)) end)\n"
        "    ov.uniforms=window:set_effect('shader_uniforms', {my_strength=0.5})\n"
        "  elseif event.step=='revoke' then\n"
        "    local operation=ov.bell:revoke()\n"
        "    assert(operation.method=='override.revoke' and ov.bell.alive)\n"
        "  else\n"
        "    assert(table.concat(ov_log, ';')=="
        "'refused:no hover:false;bell:revoked;lock:replaced;blur:window_closed', "
        "table.concat(ov_log, ';'))\n"
        "    assert(not ov.bell.alive and not ov.lock.alive and not ov.blur.alive)\n"
        "    assert(ov.corners.alive and ov.uniforms.alive)\n"
        "    assert(not pcall(function() ov.bell:on('closed', print) end))\n"
        "  end\n"
        "end)\n"
        "local grab, grab_log, refused_grab, doomed = nil, {}, nil, nil\n"
        "local function gnote(text) grab_log[#grab_log+1]=text end\n"
        "g.on('test.grab', function(event)\n"
        "  if event.step=='make' then\n"
        "    for _, bad in ipairs({0, 61, 1.5, 'x', {}, -1}) do\n"
        "      assert(not pcall(g.input.grab, {timeout=bad}))\n"
        "    end\n"
        "    assert(not pcall(g.input.grab, {bogus=1}))\n"
        "    assert(not pcall(g.input.grab, 5))\n"
        "    assert(not pcall(g.input.grab, {timeout=5}, 1))\n"
        "    grab=g.input.grab()\n"
        "    assert(grab.alive and grab.timeout==10 and grab.device=='keyboard')\n"
        "    assert(grab.id:match('^grab%-%d+$'))\n"
        "    assert(not pcall(function() grab:on('bogus', print) end))\n"
        "    grab:on('key', function(gr, key)\n"
        "      assert(gr==grab and key.name=='gnoblin.input.grab.key')\n"
        "      gnote('key:'..key.key..':'..tostring(key.text)..':'..tostring(key.pressed)..':'..\n"
        "            tostring(key.composing)..':'..key.modifiers)\n"
        "    end)\n"
        "    grab:on('closed', function(gr, reason) gnote('closed:'..tostring(reason)) end)\n"
        "    refused_grab=g.input.grab({timeout=3})\n"
        "    assert(refused_grab~=grab and refused_grab.timeout==3)\n"
        "    refused_grab:on('closed', function(gr, reason)\n"
        "      gnote('refused:'..tostring(reason)..':'..tostring(gr.alive))\n"
        "    end)\n"
        "  elseif event.step=='renew' then\n"
        "    assert(grab:renew().method=='input.grab.renew' and grab.alive)\n"
        "  elseif event.step=='release' then\n"
        "    assert(grab:release().method=='input.grab.release' and grab.alive)\n"
        "  elseif event.step=='doomed' then\n"
        "    doomed=g.input.grab({timeout=7})\n"
        "    doomed:on('closed', function(gr, reason) error('GRAB '..tostring(reason)) end)\n"
        "  else\n"
        "    assert(table.concat(grab_log, ';')=="
        "'refused:busy:false;key:a:a:true:false:0;key:A:A:true:false:1;"
        "key:dead_acute:nil:true:true:0;key:e:\\195\\169:true:false:0;key:a:nil:false:false:0;"
        "closed:released', "
        "table.concat(grab_log, ';'))\n"
        "    assert(not grab.alive and not refused_grab.alive)\n"
        "    assert(not pcall(function() grab:on('key', print) end))\n"
        "  end\n"
        "end)\n"
        "local live_log, proc, kept_tween = {}, nil, nil\n"
        "g.on('gnoblin.process.exited', function() end)\n"
        "g.on('test.live', function(event)\n"
        "  if event.step=='make' then\n"
        "    local t=g.tween()\n"
        "    assert(t.alive and t.status=='idle' and t.revision==1 and t.id:match('^%d+$'))\n"
        "    assert(not pcall(function() t:on('bogus', print) end))\n"
        "    local ok, err=pcall(function() t:watch('id', print) end)\n"
        "    assert(not ok and err:find('not an observable', 1, true), err)\n"
        "    ok, err=pcall(function() t:property(t, 'opacity', 0.5, 1) end)\n"
        "    assert(not ok and err:find('Window: opacity', 1, true), err)\n"
        "    local sub=t:watch('status', print)\n"
        "    sub:unsubscribe(); sub:unsubscribe()\n"
        "    assert(#g.process.list()==0 and g.process.by_id(99)==nil)\n"
        "    assert(not pcall(g.process.by_id, 'x'))\n"
        "    proc=g.process.spawn({'true'})\n"
        "    assert(proc.alive and proc.status=='starting' and proc.revision==1 and proc.id>0)\n"
        "    assert(g.process.list()[1]==proc and g.process.by_id(proc.id)==proc)\n"
        "    ok, err=pcall(function() proc:watch('alive', print) end)\n"
        "    assert(not ok and err:find('pid, status, exit_code, signal', 1, true), err)\n"
        "    assert(not pcall(function() proc:watch('bogus', print) end))\n"
        "    proc:watch('status', function(pr, v, o) live_log[#live_log+1]=o..'>'..v end)\n"
        "    proc:on('closed', function(pr, reason) "
        "live_log[#live_log+1]='closed:'..tostring(reason) end)\n"
        "    local input=g.input.on({type='key', accelerator='<Alt>F1'}, function() end)\n"
        "    local before=#g.config['input-handlers']\n"
        "    input:unsubscribe(); input:unsubscribe()\n"
        "    assert(before==1 and #(g.config['input-handlers'] or {})==0)\n"
        "  elseif event.step=='check' then\n"
        "    assert(table.concat(live_log, ';')=='starting>running;running>exited;closed:nil', "
        "table.concat(live_log, ';'))\n"
        "    assert(not proc.alive and proc.status=='exited' and proc.exit_code==0 and "
        "proc.revision==3)\n"
        "    assert(g.process.by_id(proc.id)==proc and #g.process.list()==0)\n"
        "    kept_tween=g.tween()\n"
        "    kept_tween:on('closed', function(t, reason) error('TWEEN '..reason) end)\n"
        "  end\n"
        "end)\n"
        "g.on('test.tween', function(event)\n"
        "  local playing=g.tween()\n"
        "  playing:property(event.window, 'opacity', 0.5, 20)\n"
        "  playing:on('closed', function(t, reason) error('PLAYING '..reason) end)\n"
        "  playing:play()\n"
        "end)\n"
        "g.on('test.configure_deferred', function(event)\n"
        "  local first=g.configure {cursor={size=29}}\n"
        "  first:on_complete(function()\n"
        "    local bad=g.configure {cursor={size='big'}}\n"
        "    bad:on_complete(function(_, failure)\n"
        "      error('REJECTED '..tostring(failure and failure.code))\n"
        "    end)\n"
        "  end)\n"
        "end)\n"
        "g.on('test.workspace_handle', function(event)\n"
        "  local created=g.workspaces.create('Handle', {id='handle'})\n"
        "  created:on_complete(function(workspace, err)\n"
        "    local listed=g.workspaces.by_id('handle')\n"
        "    assert(workspace==listed or (listed==nil and not event.listed), "
        "tostring(workspace)..' '..tostring(listed)..' '..tostring(err))\n"
        "    assert(workspace.alive and workspace.window_count==0 and workspace.windows==nil)\n"
        "    error('HANDLE '..workspace.id..':'..workspace.name)\n"
        "  end)\n"
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

    GVariantBuilder privacy_monitor_builder;
    g_variant_builder_init(&privacy_monitor_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&privacy_monitor_builder, "{sv}", "id", g_variant_new_string("eDP-1"));
    g_variant_builder_add(&privacy_monitor_builder, "{sv}", "available",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&privacy_monitor_builder, "{sv}", "enabled", g_variant_new_boolean(TRUE));
    g_variant_builder_add(&privacy_monitor_builder, "{sv}", "locked", g_variant_new_boolean(FALSE));
    GVariantBuilder privacy_monitors_builder;
    g_variant_builder_init(&privacy_monitors_builder, G_VARIANT_TYPE("av"));
    g_variant_builder_add_value(
        &privacy_monitors_builder,
        g_variant_new_variant(g_variant_builder_end(&privacy_monitor_builder)));
    GVariantBuilder monitor_privacy_snapshot_builder;
    g_variant_builder_init(&monitor_privacy_snapshot_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&monitor_privacy_snapshot_builder, "{sv}", "requested_enabled",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&monitor_privacy_snapshot_builder, "{sv}", "source",
                          g_variant_new_string("runtime"));
    g_variant_builder_add(&monitor_privacy_snapshot_builder, "{sv}", "monitors",
                          g_variant_builder_end(&privacy_monitors_builder));
    g_autoptr(GVariant) monitor_privacy_snapshot =
        g_variant_ref_sink(g_variant_builder_end(&monitor_privacy_snapshot_builder));
    gnoblin_config_update_monitor_privacy_screen_snapshot(monitor_privacy_snapshot, 12);
    g_autoptr(GVariant) monitor_privacy_read =
        gnoblin_config_read_api("monitors.privacy_screen", api_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(monitor_privacy_read);
    gboolean requested_enabled = FALSE;
    gint64 privacy_revision = 0;
    const char* privacy_source = NULL;
    g_assert_true(
        g_variant_lookup(monitor_privacy_read, "requested_enabled", "b", &requested_enabled));
    g_assert_true(g_variant_lookup(monitor_privacy_read, "source", "&s", &privacy_source));
    g_assert_true(g_variant_lookup(monitor_privacy_read, "revision", "x", &privacy_revision));
    g_assert_true(requested_enabled);
    g_assert_cmpstr(privacy_source, ==, "runtime");
    g_assert_cmpint(privacy_revision, ==, 12);
    g_autoptr(GVariant) privacy_monitors =
        g_variant_lookup_value(monitor_privacy_read, "monitors", G_VARIANT_TYPE("av"));
    g_assert_nonnull(privacy_monitors);
    g_assert_cmpuint(g_variant_n_children(privacy_monitors), ==, 1);

    GVariantBuilder set_privacy_arguments_builder;
    g_variant_builder_init(&set_privacy_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&set_privacy_arguments_builder, "{sv}", "value",
                          g_variant_new_string("inherit"));
    g_autoptr(GVariant) set_privacy_arguments =
        g_variant_ref_sink(g_variant_builder_end(&set_privacy_arguments_builder));
    g_autoptr(GVariant) set_privacy_operation =
        gnoblin_config_call_api("monitors.set_privacy_screen", set_privacy_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(set_privacy_operation);
    g_autoptr(GVariant) set_privacy_method =
        g_variant_lookup_value(set_privacy_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(set_privacy_method);
    g_assert_cmpstr(g_variant_get_string(set_privacy_method, NULL), ==,
                    "monitors.set_privacy_screen");
    g_autoptr(GVariant) set_privacy_action_arguments =
        g_variant_lookup_value(set_privacy_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    const char* set_privacy_value = NULL;
    g_assert_true(
        g_variant_lookup(set_privacy_action_arguments, "value", "&s", &set_privacy_value));
    g_assert_cmpstr(set_privacy_value, ==, "inherit");

    GVariantBuilder set_privacy_boolean_builder;
    g_variant_builder_init(&set_privacy_boolean_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&set_privacy_boolean_builder, "{sv}", "value",
                          g_variant_new_boolean(FALSE));
    g_autoptr(GVariant) set_privacy_boolean_arguments =
        g_variant_ref_sink(g_variant_builder_end(&set_privacy_boolean_builder));
    g_autoptr(GVariant) set_privacy_boolean_operation = gnoblin_config_call_api(
        "monitors.set_privacy_screen", set_privacy_boolean_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(set_privacy_boolean_operation);
    g_autoptr(GVariant) set_privacy_boolean_action_arguments =
        g_variant_lookup_value(set_privacy_boolean_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    gboolean set_privacy_enabled = TRUE;
    g_assert_true(
        g_variant_lookup(set_privacy_boolean_action_arguments, "value", "b", &set_privacy_enabled));
    g_assert_false(set_privacy_enabled);

    GVariantBuilder invalid_privacy_builder;
    g_variant_builder_init(&invalid_privacy_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&invalid_privacy_builder, "{sv}", "value", g_variant_new_string("yes"));
    g_autoptr(GVariant) invalid_privacy_arguments =
        g_variant_ref_sink(g_variant_builder_end(&invalid_privacy_builder));
    g_autoptr(GVariant) invalid_privacy_operation =
        gnoblin_config_call_api("monitors.set_privacy_screen", invalid_privacy_arguments, &error);
    g_assert_null(invalid_privacy_operation);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_clear_error(&error);
    g_autoptr(GVariant) privacy_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(privacy_operations), ==, 2);

    /* The fixed 120 second sample is gone from Lua: gnoblin.idle.watch replaces it. */
    g_autoptr(GVariant) session_activity =
        gnoblin_config_read_api("session.activity", api_arguments, &error);
    g_assert_null(session_activity);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_clear_error(&error);

    g_autoptr(GVariant) api_operation =
        gnoblin_config_call_api("workspace.list", api_arguments, &error);
    g_assert_null(api_operation);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_cmpstr(error->message, ==, "unknown Gnoblin API method 'workspace.list'");
    g_clear_error(&error);
    g_autoptr(GVariant) legacy_window_list =
        gnoblin_config_call_api("window.list", api_arguments, &error);
    g_assert_null(legacy_window_list);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_assert_cmpstr(error->message, ==, "unknown Gnoblin API method 'window.list'");
    g_clear_error(&error);
    g_autoptr(GVariant) socket_workspace_operation =
        gnoblin_config_call_api("workspace.next", api_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(socket_workspace_operation);
    g_autoptr(GVariant) socket_workspace_method =
        g_variant_lookup_value(socket_workspace_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(socket_workspace_method);
    g_assert_cmpstr(g_variant_get_string(socket_workspace_method, NULL), ==, "workspace.next");
    GVariantBuilder socket_window_arguments_builder;
    g_variant_builder_init(&socket_window_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&socket_window_arguments_builder, "{sv}", "id",
                          g_variant_new_string("42"));
    g_variant_builder_add(&socket_window_arguments_builder, "{sv}", "enabled",
                          g_variant_new_boolean(TRUE));
    g_autoptr(GVariant) socket_window_arguments =
        g_variant_ref_sink(g_variant_builder_end(&socket_window_arguments_builder));
    g_autoptr(GVariant) socket_window_operation =
        gnoblin_config_call_api("window.set_above", socket_window_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(socket_window_operation);
    g_autoptr(GVariant) socket_window_method =
        g_variant_lookup_value(socket_window_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(socket_window_method);
    g_assert_cmpstr(g_variant_get_string(socket_window_method, NULL), ==, "window.set_above");
    GVariantBuilder socket_input_arguments_builder;
    g_variant_builder_init(&socket_input_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&socket_input_arguments_builder, "{sv}", "type",
                          g_variant_new_string("xkb"));
    g_variant_builder_add(&socket_input_arguments_builder, "{sv}", "id",
                          g_variant_new_string("us"));
    g_autoptr(GVariant) socket_input_arguments =
        g_variant_ref_sink(g_variant_builder_end(&socket_input_arguments_builder));
    g_autoptr(GVariant) socket_input_operation =
        gnoblin_config_call_api("input.select", socket_input_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(socket_input_operation);
    g_autoptr(GVariant) socket_input_method =
        g_variant_lookup_value(socket_input_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(socket_input_method);
    g_assert_cmpstr(g_variant_get_string(socket_input_method, NULL), ==, "input.select");
    GVariantBuilder invalid_location_arguments_builder;
    g_variant_builder_init(&invalid_location_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&invalid_location_arguments_builder, "{sv}", "request_id",
                          g_variant_new_int64(7));
    g_variant_builder_add(&invalid_location_arguments_builder, "{sv}", "allow",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&invalid_location_arguments_builder, "{sv}", "accuracy",
                          g_variant_new_int64(3));
    g_autoptr(GVariant) invalid_location_arguments =
        g_variant_ref_sink(g_variant_builder_end(&invalid_location_arguments_builder));
    g_autoptr(GVariant) invalid_location_operation =
        gnoblin_config_call_api("location.authorize_app", invalid_location_arguments, &error);
    g_assert_null(invalid_location_operation);
    g_assert_error(error, G_FILE_ERROR, G_FILE_ERROR_INVAL);
    g_clear_error(&error);
    GVariantBuilder location_arguments_builder;
    g_variant_builder_init(&location_arguments_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&location_arguments_builder, "{sv}", "request_id",
                          g_variant_new_int64(7));
    g_variant_builder_add(&location_arguments_builder, "{sv}", "allow",
                          g_variant_new_boolean(TRUE));
    g_variant_builder_add(&location_arguments_builder, "{sv}", "accuracy", g_variant_new_int64(5));
    g_autoptr(GVariant) location_arguments =
        g_variant_ref_sink(g_variant_builder_end(&location_arguments_builder));
    g_autoptr(GVariant) location_operation =
        gnoblin_config_call_api("location.authorize_app", location_arguments, &error);
    g_assert_no_error(error);
    g_assert_nonnull(location_operation);
    g_autoptr(GVariant) location_method =
        g_variant_lookup_value(location_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_nonnull(location_method);
    g_assert_cmpstr(g_variant_get_string(location_method, NULL), ==, "location.authorize_app");
    g_autoptr(GVariant) queued_api_operations = gnoblin_config_drain_runtime_operations();
    g_assert_cmpuint(g_variant_n_children(queued_api_operations), ==, 4);
    g_autoptr(GVariant) queued_api_operation = g_variant_get_child_value(queued_api_operations, 0);
    g_autoptr(GVariant) queued_window_operation =
        g_variant_get_child_value(queued_api_operations, 1);
    g_autoptr(GVariant) queued_input_operation =
        g_variant_get_child_value(queued_api_operations, 2);
    g_autoptr(GVariant) queued_location_operation =
        g_variant_get_child_value(queued_api_operations, 3);
    g_assert_true(g_variant_equal(socket_workspace_operation, queued_api_operation));
    g_assert_true(g_variant_equal(socket_window_operation, queued_window_operation));
    g_assert_true(g_variant_equal(socket_input_operation, queued_input_operation));
    g_assert_true(g_variant_equal(location_operation, queued_location_operation));

    const char* queued_methods[] = {"session.suspend",  "session.hibernate",      "session.reboot",
                                    "session.poweroff", "config.restore_default", NULL};
    for (guint i = 0; queued_methods[i]; i++) {
        g_autoptr(GVariant) queued =
            gnoblin_config_call_api(queued_methods[i], api_arguments, &error);
        g_assert_no_error(error);
        g_assert_nonnull(queued);
        g_autoptr(GVariant) queued_batch = gnoblin_config_drain_runtime_operations();
        g_assert_cmpuint(g_variant_n_children(queued_batch), ==, 1);
        g_autoptr(GVariant) queued_record = g_variant_get_child_value(queued_batch, 0);
        const char* queued_method = NULL;
        g_assert_true(g_variant_lookup(queued_record, "method", "&s", &queued_method));
        g_assert_cmpstr(queued_method, ==, queued_methods[i]);
    }

    GVariantBuilder thumbnail_event_builder;
    g_variant_builder_init(&thumbnail_event_builder, G_VARIANT_TYPE_VARDICT);
    GVariantBuilder thumbnail_window_builder;
    g_variant_builder_init(&thumbnail_window_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&thumbnail_window_builder, "{sv}", "id", g_variant_new_string("42"));
    GVariantBuilder thumbnail_frame_builder;
    g_variant_builder_init(&thumbnail_frame_builder, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&thumbnail_window_builder, "{sv}", "frame",
                          g_variant_builder_end(&thumbnail_frame_builder));
    g_variant_builder_add(&thumbnail_event_builder, "{sv}", "window",
                          g_variant_builder_end(&thumbnail_window_builder));
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

    GVariantBuilder menu_window;
    g_variant_builder_init(&menu_window, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&menu_window, "{sv}", "id", g_variant_new_string("42"));
    GVariantBuilder menu_frame;
    g_variant_builder_init(&menu_frame, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&menu_frame, "{sv}", "x", g_variant_new_int64(0));
    g_variant_builder_add(&menu_frame, "{sv}", "y", g_variant_new_int64(0));
    g_variant_builder_add(&menu_frame, "{sv}", "width", g_variant_new_int64(800));
    g_variant_builder_add(&menu_frame, "{sv}", "height", g_variant_new_int64(600));
    g_variant_builder_add(&menu_window, "{sv}", "frame", g_variant_builder_end(&menu_frame));
    GVariantBuilder menu_windows;
    g_variant_builder_init(&menu_windows, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&menu_windows, "v", g_variant_builder_end(&menu_window));
    GVariantBuilder menu_snapshot;
    g_variant_builder_init(&menu_snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&menu_snapshot, "{sv}", "windows", g_variant_builder_end(&menu_windows));
    g_autoptr(GVariant) menu_snapshot_value =
        g_variant_ref_sink(g_variant_builder_end(&menu_snapshot));
    gnoblin_config_update_window_snapshot(menu_snapshot_value, 1);
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
    g_autoptr(GVariant) malformed_method =
        g_variant_lookup_value(malformed_operation, "method", G_VARIANT_TYPE_STRING);
    g_assert_cmpstr(g_variant_get_string(malformed_method, NULL), ==, "window.begin_move");
    g_autoptr(GVariant) malformed_arguments =
        g_variant_lookup_value(malformed_operation, "arguments", G_VARIANT_TYPE_VARDICT);
    /* A bad edge is a Lua error and spends nothing: the grant then starts the move. */
    g_assert_null(g_variant_lookup_value(malformed_arguments, "_malformed", NULL));
    g_autoptr(GVariant) malformed_handle =
        g_variant_lookup_value(malformed_arguments, "_menu_context_handle", G_VARIANT_TYPE_UINT64);
    g_assert_nonnull(malformed_handle);
    g_assert_cmpuint(g_variant_get_uint64(malformed_handle), ==, 78);
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

    /* Outputs are live objects: one handle per connector that watches and signals follow. */
    GVariantBuilder outputs;
    g_variant_builder_init(&outputs, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&outputs, "v", test_output("eDP-1", TRUE, 1.0, TRUE));
    g_variant_builder_add(&outputs, "v", test_output("HDMI-1", FALSE, 1.0, FALSE));
    GVariantBuilder output_snapshot;
    g_variant_builder_init(&output_snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&output_snapshot, "{sv}", "outputs", g_variant_builder_end(&outputs));
    g_autoptr(GVariant) output_snapshot_value =
        g_variant_ref_sink(g_variant_builder_end(&output_snapshot));
    gnoblin_config_update_output_snapshot(output_snapshot_value, 3);
    GVariantBuilder read_step;
    g_variant_builder_init(&read_step, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&read_step, "{sv}", "step", g_variant_new_string("read"));
    g_assert_cmpuint(
        dispatch_counting_operations("test.outputs", g_variant_builder_end(&read_step)), ==, 1);
    GVariantBuilder output_changed;
    g_variant_builder_init(&output_changed, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&output_changed, "{sv}", "output_id", g_variant_new_string("eDP-1"));
    g_variant_builder_add(&output_changed, "{sv}", "output", test_output("eDP-1", TRUE, 2.0, TRUE));
    const char* changed_fields[] = {"scale", NULL};
    g_variant_builder_add(&output_changed, "{sv}", "changed",
                          g_variant_new_strv(changed_fields, -1));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.output.changed",
                                                  g_variant_builder_end(&output_changed)),
                     ==, 0);
    GVariantBuilder output_removed;
    g_variant_builder_init(&output_removed, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&output_removed, "{sv}", "output_id", g_variant_new_string("eDP-1"));
    g_variant_builder_add(&output_removed, "{sv}", "last", test_output("eDP-1", TRUE, 2.0, TRUE));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.output.removed",
                                                  g_variant_builder_end(&output_removed)),
                     ==, 0);
    GVariantBuilder check_step;
    g_variant_builder_init(&check_step, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&check_step, "{sv}", "step", g_variant_new_string("check"));
    g_assert_cmpuint(
        dispatch_counting_operations("test.outputs", g_variant_builder_end(&check_step)), ==, 1);

    /* The privacy screen request belongs to the session: with no capable output, true and false
     * fail and "inherit" is still accepted. */
    GVariantBuilder bare_outputs;
    g_variant_builder_init(&bare_outputs, G_VARIANT_TYPE("av"));
    g_variant_builder_add(&bare_outputs, "v", test_output("HDMI-1", TRUE, 1.0, FALSE));
    GVariantBuilder bare_snapshot;
    g_variant_builder_init(&bare_snapshot, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&bare_snapshot, "{sv}", "outputs", g_variant_builder_end(&bare_outputs));
    g_autoptr(GVariant) bare_snapshot_value =
        g_variant_ref_sink(g_variant_builder_end(&bare_snapshot));
    gnoblin_config_update_output_snapshot(bare_snapshot_value, 4);
    GVariantBuilder no_privacy_step;
    g_variant_builder_init(&no_privacy_step, G_VARIANT_TYPE_VARDICT);
    g_assert_cmpuint(
        dispatch_counting_operations("test.no-privacy", g_variant_builder_end(&no_privacy_step)),
        ==, 1);

    /* gnoblin.find selects live objects by their declared fields. The output snapshot above was
     * replaced by one with a single output, and the window snapshot holds window 42. */
    g_assert_cmpuint(dispatch_counting_operations("test.find", step_payload("run")), ==, 1);
    g_autofree char* find_error = gnoblin_config_take_runtime_callback_error();
    g_assert_null(find_error);

    /* Configuration is changed by calls that return an Operation, not by assignment. At run time
     * gnoblin.configure finishes its Operation when the change is committed, or fails it when the
     * change is rejected. */
    g_assert_cmpuint(dispatch_counting_operations("test.configure", step_payload("start")), ==, 0);
    run_deferred_callbacks();
    g_autoptr(GVariant) reject_payload = g_variant_ref_sink(step_payload("reject"));
    g_autoptr(GVariant) rejected =
        gnoblin_config_dispatch_event("test.configure", reject_payload, &error);
    g_assert_null(rejected);
    g_assert_nonnull(error);
    g_clear_error(&error);
    gnoblin_config_finish_event(FALSE);
    run_deferred_callbacks();
    g_assert_cmpuint(dispatch_counting_operations("test.configure", step_payload("check")), ==, 1);
    g_autofree char* configure_error = gnoblin_config_take_runtime_callback_error();
    g_assert_null(configure_error);

    /* Launches are live objects: the compositor's events move one handle per token. */
    g_autoptr(GArray) launch_ids = dispatch_operation_ids("test.launches", step_payload("begin"));
    g_assert_cmpuint(launch_ids->len, ==, 4);
    GVariantBuilder launch_record;
    g_variant_builder_init(&launch_record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&launch_record, "{sv}", "token", g_variant_new_string("tok"));
    g_variant_builder_add(&launch_record, "{sv}", "application",
                          g_variant_new_string("org.example.Editor"));
    g_variant_builder_add(&launch_record, "{sv}", "started_at", g_variant_new_int64(1720000000123));
    g_variant_builder_add(&launch_record, "{sv}", "timeout_ms", g_variant_new_int64(500));
    g_variant_builder_add(&launch_record, "{sv}", "state", g_variant_new_string("started"));
    g_variant_builder_add(&launch_record, "{sv}", "revision", g_variant_new_int64(2));
    GVariantBuilder launch_changed;
    g_variant_builder_init(&launch_changed, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&launch_changed, "{sv}", "launch", g_variant_builder_end(&launch_record));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.launch.changed",
                                                  g_variant_builder_end(&launch_changed)),
                     ==, 0);
    g_assert_cmpuint(dispatch_counting_operations("test.launches", step_payload("check")), ==, 1);

    /* Shortcuts: one handle per binding; activation and the session reach its signals, a refused
     * binding ends with the reason, and unbinding ends it when the compositor confirms. */
    g_autoptr(GArray) shortcut_ids = dispatch_operation_ids("test.shortcuts", step_payload("bind"));
    g_assert_cmpuint(shortcut_ids->len, ==, 4);
    complete_operation(g_array_index(shortcut_ids, gint64, 3), FALSE, NULL, "taken");
    GVariantBuilder bound_record;
    g_variant_builder_init(&bound_record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&bound_record, "{sv}", "id", g_variant_new_string("sw"));
    g_variant_builder_add(&bound_record, "{sv}", "accelerator",
                          g_variant_new_string("<Super>space"));
    g_variant_builder_add(&bound_record, "{sv}", "hold", g_variant_new_string("super"));
    g_variant_builder_add(&bound_record, "{sv}", "trigger", g_variant_new_string("press"));
    g_variant_builder_add(&bound_record, "{sv}", "mode", g_variant_new_string("modal"));
    g_variant_builder_add(&bound_record, "{sv}", "capture_input", g_variant_new_boolean(FALSE));
    complete_operation(g_array_index(shortcut_ids, gint64, 0), TRUE,
                       g_variant_builder_end(&bound_record), NULL);
    GVariantBuilder activated;
    g_variant_builder_init(&activated, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&activated, "{sv}", "id", g_variant_new_string("sw"));
    g_variant_builder_add(&activated, "{sv}", "accelerator", g_variant_new_string("<Super>space"));
    g_variant_builder_add(&activated, "{sv}", "first", g_variant_new_boolean(TRUE));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.shortcut.binding-activated",
                                                  g_variant_builder_end(&activated)),
                     ==, 0);
    GVariantBuilder session_started;
    g_variant_builder_init(&session_started, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&session_started, "{sv}", "id", g_variant_new_string("sw"));
    g_variant_builder_add(&session_started, "{sv}", "session_id", g_variant_new_uint64(3));
    g_variant_builder_add(&session_started, "{sv}", "first", g_variant_new_boolean(TRUE));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.shortcut.session.activated",
                                                  g_variant_builder_end(&session_started)),
                     ==, 0);
    GVariantBuilder session_ended;
    g_variant_builder_init(&session_ended, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&session_ended, "{sv}", "id", g_variant_new_string("sw"));
    g_variant_builder_add(&session_ended, "{sv}", "session_id", g_variant_new_uint64(3));
    g_variant_builder_add(&session_ended, "{sv}", "reason", g_variant_new_string("cancelled"));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.shortcut.session.ended",
                                                  g_variant_builder_end(&session_ended)),
                     ==, 0);
    GVariantBuilder unbound;
    g_variant_builder_init(&unbound, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&unbound, "{sv}", "id", g_variant_new_string("sw"));
    g_variant_builder_add(&unbound, "{sv}", "unbound", g_variant_new_boolean(TRUE));
    complete_operation(g_array_index(shortcut_ids, gint64, 2), TRUE,
                       g_variant_builder_end(&unbound), NULL);
    g_assert_cmpuint(dispatch_counting_operations("test.shortcuts", step_payload("check")), ==, 1);

    /* Idle watches: one object for each watch; the compositor's idle and active events move it,
     * a refused watch ends with the reason, and cancelling ends it when the compositor confirms. */
    g_autoptr(GArray) idle_ids = dispatch_operation_ids("test.idle", step_payload("watch"));
    g_assert_cmpuint(idle_ids->len, ==, 3);
    complete_operation(g_array_index(idle_ids, gint64, 2), FALSE, NULL, "full");
    g_autofree char* watch_id =
        g_strdup_printf("idle-%" G_GINT64_FORMAT, g_array_index(idle_ids, gint64, 0));
    GVariantBuilder watch_record;
    g_variant_builder_init(&watch_record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&watch_record, "{sv}", "id", g_variant_new_string(watch_id));
    g_variant_builder_add(&watch_record, "{sv}", "seconds", g_variant_new_double(90));
    g_variant_builder_add(&watch_record, "{sv}", "idle", g_variant_new_boolean(FALSE));
    complete_operation(g_array_index(idle_ids, gint64, 0), TRUE,
                       g_variant_builder_end(&watch_record), NULL);
    for (guint step = 0; step < 2; step++) {
        GVariantBuilder idle_record, idle_event;
        g_variant_builder_init(&idle_record, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&idle_record, "{sv}", "id", g_variant_new_string(watch_id));
        g_variant_builder_add(&idle_record, "{sv}", "seconds", g_variant_new_double(90));
        g_variant_builder_add(&idle_record, "{sv}", "idle", g_variant_new_boolean(step == 0));
        g_variant_builder_init(&idle_event, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&idle_event, "{sv}", "watch", g_variant_builder_end(&idle_record));
        g_assert_cmpuint(dispatch_counting_operations(step == 0 ? "gnoblin.idle.watch-idle"
                                                                : "gnoblin.idle.watch-active",
                                                      g_variant_builder_end(&idle_event)),
                         ==, 0);
    }
    g_autoptr(GArray) cancel_ids = dispatch_operation_ids("test.idle", step_payload("cancel"));
    g_assert_cmpuint(cancel_ids->len, ==, 1);
    GVariantBuilder cancelled;
    g_variant_builder_init(&cancelled, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&cancelled, "{sv}", "cancelled", g_variant_new_boolean(TRUE));
    complete_operation(g_array_index(cancel_ids, gint64, 0), TRUE,
                       g_variant_builder_end(&cancelled), NULL);
    g_assert_cmpuint(dispatch_counting_operations("test.idle", step_payload("check")), ==, 0);

    /* Overrides: a setting path or a window effect makes one object; the compositor ends it with
     * a reason, and a committed reload ends the ones left. */
    GVariantBuilder override_window, override_frame, override_make;
    g_variant_builder_init(&override_frame, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_init(&override_window, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&override_window, "{sv}", "id", g_variant_new_string("42"));
    g_variant_builder_add(&override_window, "{sv}", "frame",
                          g_variant_builder_end(&override_frame));
    g_variant_builder_init(&override_make, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&override_make, "{sv}", "step", g_variant_new_string("make"));
    g_variant_builder_add(&override_make, "{sv}", "window",
                          g_variant_builder_end(&override_window));
    g_autoptr(GVariant) override_make_payload =
        g_variant_ref_sink(g_variant_builder_end(&override_make));
    g_autoptr(GVariant) override_make_result =
        gnoblin_config_dispatch_event("test.overrides", override_make_payload, &error);
    g_assert_no_error(error);
    g_assert_nonnull(override_make_result);
    g_autoptr(GVariant) override_operations = gnoblin_config_drain_runtime_operations();
    gnoblin_config_finish_event(TRUE);
    g_assert_cmpuint(g_variant_n_children(override_operations), ==, 6);
    g_autoptr(GArray) override_ids = g_array_new(FALSE, FALSE, sizeof(gint64));
    const char* const expected_methods[] = {"settings.override", "settings.override",
                                            "settings.override", "window.set_effect",
                                            "window.set_effect", "window.set_effect"};
    for (gsize i = 0; i < 6; i++) {
        g_autoptr(GVariant) operation = g_variant_get_child_value(override_operations, i);
        const char* method = NULL;
        gint64 id = 0;
        g_assert_true(g_variant_lookup(operation, "method", "&s", &method));
        g_assert_true(g_variant_lookup(operation, "request_id", "x", &id));
        g_assert_cmpstr(method, ==, expected_methods[i]);
        g_array_append_val(override_ids, id);
        g_autoptr(GVariant) arguments =
            g_variant_lookup_value(operation, "arguments", G_VARIANT_TYPE_VARDICT);
        const char* text = NULL;
        if (i < 3) {
            g_assert_true(g_variant_lookup(arguments, "section", "&s", &text));
            g_assert_cmpstr(text, ==,
                            i == 0   ? "compositor"
                            : i == 1 ? "input"
                                     : "window-management");
            g_assert_true(g_variant_lookup(arguments, "key", "&s", &text));
            g_assert_cmpstr(text, ==,
                            i == 0   ? "visual-bell"
                            : i == 1 ? "orientation-lock"
                                     : "focus-mode");
        } else {
            g_assert_true(g_variant_lookup(arguments, "id", "&s", &text));
            g_assert_cmpstr(text, ==, "42");
        }
        if (i == 4) {
            /* The corner table reaches the compositor with the keys the rule engine reads. */
            g_autoptr(GVariant) corners = g_variant_lookup_value(arguments, "value", NULL);
            gint64 radius = 0, border = 0;
            g_assert_true(g_variant_lookup(corners, "radius", "x", &radius));
            g_assert_true(g_variant_lookup(corners, "border-width", "x", &border));
            g_assert_cmpint(radius, ==, 12);
            g_assert_cmpint(border, ==, 2);
        }
        if (i == 5) {
            /* Uniform names are data, not settings: the underscore stays. */
            g_autoptr(GVariant) uniforms = g_variant_lookup_value(arguments, "value", NULL);
            double strength = 0;
            g_assert_true(g_variant_lookup(uniforms, "my_strength", "d", &strength));
            g_assert_cmpfloat(strength, ==, 0.5);
        }
    }
    complete_operation(g_array_index(override_ids, gint64, 2), FALSE, NULL, "no hover");
    g_autofree char* override_id_bell =
        g_strdup_printf("override-%" G_GINT64_FORMAT, g_array_index(override_ids, gint64, 0));
    g_autofree char* override_id_lock =
        g_strdup_printf("override-%" G_GINT64_FORMAT, g_array_index(override_ids, gint64, 1));
    g_autofree char* override_id_blur =
        g_strdup_printf("override-%" G_GINT64_FORMAT, g_array_index(override_ids, gint64, 3));
    g_autoptr(GArray) revoke_ids = dispatch_operation_ids("test.overrides", step_payload("revoke"));
    g_assert_cmpuint(revoke_ids->len, ==, 1);
    const char* const closings[][2] = {
        {"bell", "revoked"}, {"lock", "replaced"}, {"blur", "window_closed"}};
    const char* const closing_ids[] = {override_id_bell, override_id_lock, override_id_blur};
    for (guint i = 0; i < G_N_ELEMENTS(closings); i++) {
        GVariantBuilder closed;
        g_variant_builder_init(&closed, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&closed, "{sv}", "id", g_variant_new_string(closing_ids[i]));
        g_variant_builder_add(&closed, "{sv}", "reason", g_variant_new_string(closings[i][1]));
        g_assert_cmpuint(
            dispatch_counting_operations("gnoblin.override.closed", g_variant_builder_end(&closed)),
            ==, 0);
    }
    g_assert_cmpuint(dispatch_counting_operations("test.overrides", step_payload("check")), ==, 0);

    /* A reload that commits ends the overrides that remain, in the old runtime, before it goes. */
    g_free(gnoblin_config_take_runtime_callback_error());
    g_autoptr(GVariant) override_reloaded = gnoblin_config_load_runtime(root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(override_reloaded);
    gnoblin_config_finish_load(TRUE);
    g_autofree char* reload_error = gnoblin_config_take_runtime_callback_error();
    g_assert_nonnull(reload_error);
    g_assert_nonnull(strstr(reload_error, "MARK config_reloaded"));

    /* Keyboard grabs: one object for each grab; the compositor's key events reach its "key"
     * listeners, a refused grab ends with the reason, and the compositor ends the rest. */
    g_autoptr(GArray) grab_ids = dispatch_operation_ids("test.grab", step_payload("make"));
    g_assert_cmpuint(grab_ids->len, ==, 2);
    complete_operation(g_array_index(grab_ids, gint64, 1), FALSE, NULL, "busy");
    g_autofree char* grab_id =
        g_strdup_printf("grab-%" G_GINT64_FORMAT, g_array_index(grab_ids, gint64, 0));
    GVariantBuilder grab_record;
    g_variant_builder_init(&grab_record, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&grab_record, "{sv}", "id", g_variant_new_string(grab_id));
    g_variant_builder_add(&grab_record, "{sv}", "timeout", g_variant_new_int64(10));
    complete_operation(g_array_index(grab_ids, gint64, 0), TRUE,
                       g_variant_builder_end(&grab_record), NULL);
    const struct {
        const char* key;
        guint32 keysym;
        guint32 modifiers;
        gboolean pressed;
        gboolean composing;
        const char* text;
    } grab_keys[] = {
        {"a", 0x61, 0, TRUE, FALSE, "a"},
        {"A", 0x41, 1, TRUE, FALSE, "A"},
        {"dead_acute", 0xfe51, 0, TRUE, TRUE, NULL},
        {"e", 0x65, 0, TRUE, FALSE, "\xc3\xa9"},
        {"a", 0x61, 0, FALSE, FALSE, NULL},
    };
    for (guint i = 0; i < G_N_ELEMENTS(grab_keys); i++) {
        GVariantBuilder key_event;
        g_variant_builder_init(&key_event, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&key_event, "{sv}", "id", g_variant_new_string(grab_id));
        g_variant_builder_add(&key_event, "{sv}", "key", g_variant_new_string(grab_keys[i].key));
        g_variant_builder_add(&key_event, "{sv}", "keysym",
                              g_variant_new_uint32(grab_keys[i].keysym));
        g_variant_builder_add(&key_event, "{sv}", "keycode", g_variant_new_uint32(38));
        g_variant_builder_add(&key_event, "{sv}", "modifiers",
                              g_variant_new_uint32(grab_keys[i].modifiers));
        g_variant_builder_add(&key_event, "{sv}", "pressed",
                              g_variant_new_boolean(grab_keys[i].pressed));
        g_variant_builder_add(&key_event, "{sv}", "repeated", g_variant_new_boolean(FALSE));
        g_variant_builder_add(&key_event, "{sv}", "composing",
                              g_variant_new_boolean(grab_keys[i].composing));
        if (grab_keys[i].text)
            g_variant_builder_add(&key_event, "{sv}", "text",
                                  g_variant_new_string(grab_keys[i].text));
        g_variant_builder_add(&key_event, "{sv}", "time", g_variant_new_int64(1000 + i));
        g_assert_cmpuint(dispatch_counting_operations("gnoblin.input.grab.key",
                                                      g_variant_builder_end(&key_event)),
                         ==, 0);
    }
    g_autoptr(GArray) renew_ids = dispatch_operation_ids("test.grab", step_payload("renew"));
    g_assert_cmpuint(renew_ids->len, ==, 1);
    g_autoptr(GArray) release_ids = dispatch_operation_ids("test.grab", step_payload("release"));
    g_assert_cmpuint(release_ids->len, ==, 1);
    GVariantBuilder grab_closed;
    g_variant_builder_init(&grab_closed, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&grab_closed, "{sv}", "id", g_variant_new_string(grab_id));
    g_variant_builder_add(&grab_closed, "{sv}", "reason", g_variant_new_string("released"));
    g_assert_cmpuint(dispatch_counting_operations("gnoblin.input.grab.closed",
                                                  g_variant_builder_end(&grab_closed)),
                     ==, 0);
    g_assert_cmpuint(dispatch_counting_operations("test.grab", step_payload("check")), ==, 0);

    /* Tweens and processes are live objects: a process moves through its states and ends once, a
     * subscription of input.on can be undone, and a reload ends a tween that is still alive. */
    g_autoptr(GArray) live_ids = dispatch_operation_ids("test.live", step_payload("make"));
    g_assert_cmpuint(live_ids->len, ==, 1);
    GVariantBuilder spawned;
    g_variant_builder_init(&spawned, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&spawned, "{sv}", "pid", g_variant_new_int64(4242));
    complete_operation(g_array_index(live_ids, gint64, 0), TRUE, g_variant_builder_end(&spawned),
                       NULL);
    g_free(gnoblin_config_take_runtime_callback_error());
    GVariantBuilder exited;
    g_variant_builder_init(&exited, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&exited, "{sv}", "id",
                          g_variant_new_int64(g_array_index(live_ids, gint64, 0)));
    g_variant_builder_add(&exited, "{sv}", "exit_code", g_variant_new_int64(0));
    g_assert_cmpuint(
        dispatch_counting_operations("gnoblin.process.exited", g_variant_builder_end(&exited)), ==,
        0);
    g_assert_cmpuint(dispatch_counting_operations("test.live", step_payload("check")), ==, 0);
    g_autoptr(GVariant) live_reloaded = gnoblin_config_load_runtime(root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(live_reloaded);
    gnoblin_config_finish_load(TRUE);
    g_autofree char* tween_reload_error = gnoblin_config_take_runtime_callback_error();
    g_assert_nonnull(tween_reload_error);
    g_assert_nonnull(strstr(tween_reload_error, "TWEEN config_reloaded"));

    /* A playing tween does not hold a reload back. The compositor cancels it when the new
     * configuration takes effect, and its Tween closes with the reason "config_reloaded". */
    for (guint round = 0; round < 3; round++) {
        GVariantBuilder tween_window, tween_frame, tween_event;
        g_variant_builder_init(&tween_frame, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_init(&tween_window, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&tween_window, "{sv}", "id", g_variant_new_string("42"));
        g_variant_builder_add(&tween_window, "{sv}", "frame", g_variant_builder_end(&tween_frame));
        g_variant_builder_init(&tween_event, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&tween_event, "{sv}", "window", g_variant_builder_end(&tween_window));
        guint pending_before = gnoblin_config_runtime_pending_operations();
        g_autoptr(GArray) play_ids =
            dispatch_operation_ids("test.tween", g_variant_builder_end(&tween_event));
        g_assert_cmpuint(play_ids->len, ==, 1);
        gint64 play_id = g_array_index(play_ids, gint64, 0);
        g_assert_cmpuint(gnoblin_config_runtime_pending_operations(), ==, pending_before);
        g_assert_true(gnoblin_config_operation_survives_reload(play_id));
        g_assert_false(gnoblin_config_operation_survives_reload(play_id + 1000));
        if (round == 2)
            break;
        /* The compositor's cancel reaches the old runtime: the reason depends on the reload. */
        gnoblin_config_reload_in_flight(round == 0);
        GVariantBuilder cancelled_tween;
        g_variant_builder_init(&cancelled_tween, G_VARIANT_TYPE_VARDICT);
        g_variant_builder_add(&cancelled_tween, "{sv}", "outcome",
                              g_variant_new_string("cancelled"));
        complete_operation(play_id, TRUE, g_variant_builder_end(&cancelled_tween), NULL);
        gnoblin_config_reload_in_flight(FALSE);
        g_autofree char* cancel_error = gnoblin_config_take_runtime_callback_error();
        g_assert_nonnull(cancel_error);
        g_assert_nonnull(
            strstr(cancel_error, round == 0 ? "PLAYING config_reloaded" : "PLAYING cancelled"));
    }
    g_autoptr(GVariant) playing_reloaded = gnoblin_config_load_runtime(root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(playing_reloaded);
    gnoblin_config_finish_load(TRUE);
    g_autofree char* playing_reload_error = gnoblin_config_take_runtime_callback_error();
    g_assert_nonnull(playing_reload_error);
    g_assert_nonnull(strstr(playing_reload_error, "PLAYING config_reloaded"));

    /* A workspace operation answers with the live handle that gnoblin.workspaces.by_id returns,
     * in the shape of the snapshot, whether or not the snapshot lists the workspace yet. */
    g_autoptr(GArray) handle_ids =
        dispatch_operation_ids("test.workspace_handle", step_payload("make"));
    g_assert_cmpuint(handle_ids->len, ==, 1);
    GVariantBuilder created_workspace, created_list, created_snapshot;
    g_variant_builder_init(&created_workspace, G_VARIANT_TYPE_VARDICT);
    g_variant_builder_add(&created_workspace, "{sv}", "id", g_variant_new_string("handle"));
    g_variant_builder_add(&created_workspace, "{sv}", "number", g_variant_new_int64(2));
    g_variant_builder_add(&created_workspace, "{sv}", "name", g_variant_new_string("Handle"));
    g_variant_builder_add(&created_workspace, "{sv}", "active", g_variant_new_boolean(FALSE));
    g_variant_builder_add(&created_workspace, "{sv}", "windows", g_variant_new_int64(0));
    g_autoptr(GVariant) created_value =
        g_variant_ref_sink(g_variant_builder_end(&created_workspace));
    for (guint listed = 0; listed < 2; listed++) {
        if (!listed) {
            g_autoptr(GVariant) empty_snapshot =
                g_variant_ref_sink(g_variant_new_parsed("{'workspaces': <@av []>}"));
            gnoblin_config_update_workspace_snapshot(empty_snapshot, 8);
        } else {
            /* The snapshot lists it: the handle is the snapshot's. */
            GVariantBuilder record;
            g_variant_builder_init(&record, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&record, "{sv}", "id", g_variant_new_string("handle"));
            g_variant_builder_add(&record, "{sv}", "number", g_variant_new_int64(2));
            g_variant_builder_add(&record, "{sv}", "name", g_variant_new_string("Handle"));
            g_variant_builder_add(&record, "{sv}", "active", g_variant_new_boolean(FALSE));
            g_variant_builder_add(&record, "{sv}", "window_count", g_variant_new_int64(0));
            g_variant_builder_add(&record, "{sv}", "revision", g_variant_new_int64(9));
            g_variant_builder_init(&created_list, G_VARIANT_TYPE("av"));
            g_variant_builder_add_value(&created_list,
                                        g_variant_new_variant(g_variant_builder_end(&record)));
            g_variant_builder_init(&created_snapshot, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&created_snapshot, "{sv}", "workspaces",
                                  g_variant_builder_end(&created_list));
            g_autoptr(GVariant) snapshot =
                g_variant_ref_sink(g_variant_builder_end(&created_snapshot));
            gnoblin_config_update_workspace_snapshot(snapshot, 9);
            GVariantBuilder listed_payload;
            g_variant_builder_init(&listed_payload, G_VARIANT_TYPE_VARDICT);
            g_variant_builder_add(&listed_payload, "{sv}", "listed", g_variant_new_boolean(TRUE));
            handle_ids = dispatch_operation_ids("test.workspace_handle",
                                                g_variant_builder_end(&listed_payload));
            g_assert_cmpuint(handle_ids->len, ==, 1);
        }
        complete_operation(g_array_index(handle_ids, gint64, 0), TRUE, g_variant_ref(created_value),
                           NULL);
        g_autofree char* handle_error = gnoblin_config_take_runtime_callback_error();
        g_assert_nonnull(handle_error);
        g_assert_nonnull(strstr(handle_error, "HANDLE handle:Handle"));
    }

    /* A configure call in an Operation callback that the validator rejects fails its Operation
     * and nothing else: the callback did not fail. */
    g_autoptr(GArray) deferred_ids =
        dispatch_operation_ids("test.configure_deferred", step_payload("make"));
    g_assert_cmpuint(deferred_ids->len, ==, 0);
    for (guint pass = 0; pass < 2; pass++) {
        g_autoptr(GError) deferred_error = NULL;
        g_autoptr(GVariant) deferred_document =
            gnoblin_config_dispatch_deferred_callbacks(&deferred_error);
        g_assert_no_error(deferred_error);
        g_assert_null(deferred_document);
    }
    g_autofree char* deferred_callback_error = gnoblin_config_take_runtime_callback_error();
    g_assert_nonnull(deferred_callback_error);
    g_assert_nonnull(strstr(deferred_callback_error, "REJECTED invalid_argument"));

    /* A worker that shuts down in order tells the grab that is left; so does a reload. */
    g_autoptr(GArray) stop_ids = dispatch_operation_ids("test.grab", step_payload("doomed"));
    g_assert_cmpuint(stop_ids->len, ==, 1);
    g_free(gnoblin_config_take_runtime_callback_error());
    gnoblin_config_stop_runtime();
    g_autofree char* stop_error = gnoblin_config_take_runtime_callback_error();
    g_assert_nonnull(stop_error);
    g_assert_nonnull(strstr(stop_error, "GRAB runtime_stopped"));
    g_autoptr(GArray) reload_ids = dispatch_operation_ids("test.grab", step_payload("doomed"));
    g_assert_cmpuint(reload_ids->len, ==, 1);
    g_autoptr(GVariant) grab_reloaded = gnoblin_config_load_runtime(root, NULL, NULL, &error);
    g_assert_no_error(error);
    g_assert_nonnull(grab_reloaded);
    gnoblin_config_finish_load(TRUE);
    g_autofree char* grab_reload_error = gnoblin_config_take_runtime_callback_error();
    g_assert_nonnull(grab_reload_error);
    g_assert_nonnull(strstr(grab_reload_error, "GRAB config_reloaded"));

    g_unlink(root);
    g_unlink(component);
    g_rmdir(directory);
    g_print("PASS: declarations, named overrides, snake_case, imports and reloads\n");
    return 0;
}
