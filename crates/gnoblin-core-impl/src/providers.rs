//! `provider_register` and `setting_set`: the ABI face of the compositor's provider slots.
//!
//! A library that is not the core (std) registers the desktop policy the compositor asks it for as C vtables
//! (`design/modular-framework.md`, 9.4 and 28). This module wraps each vtable as the Rust trait the compositor drives
//! (`gnoblin-provider-wire`), puts it in the process's [`ProviderSlots`] and ties its life to the generation that registered
//! it: the registration is a host registration like a timer, so retiring the generation, or `remove(reg)`, ends it, and the
//! compositor falls back (`NullWm`, `no-ui`, no observer).
//!
//! The slots are the process's: [`slots`] is what `gnoblin_main` hands the compositor, so a registration made before the
//! session serves takes effect when it does.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use gnoblin_compositor::providers::{ConfigObserver, ProviderId, ProviderSlots, SettingError, SettingValue};
use gnoblin_core_abi::{
    self as abi, GnoblinConfigVtable, GnoblinCtx, GnoblinDropFn, GnoblinReg, GnoblinUiVtable, GnoblinValue,
    GnoblinWmVtable,
};
use gnoblin_provider_wire::{ForeignConfig, ForeignUi, ForeignWm, Report};
use gnoblin_shell_host::handle::{KIND_REG, encode};

use crate::guard::set_code_error;
use crate::log;
use crate::registry::{self, Subscription};

/// The slots of this process.
static SLOTS: LazyLock<ProviderSlots> = LazyLock::new(ProviderSlots::new);

/// The provider slots of the process: registered into by the table, read by the compositor.
#[must_use]
pub fn slots() -> ProviderSlots {
    SLOTS.clone()
}

/// Says what a provider that failed did, in the core's log, once per call of the adapter.
fn report(kind: &'static str) -> Report {
    Box::new(move |line| log::warn!(area = "providers", kind, "{line}"))
}

/// A configuration observer that is a [`ForeignConfig`].
struct DocumentWatcher(ForeignConfig);

impl ConfigObserver for DocumentWatcher {
    fn apply(&mut self, document: &str) {
        self.0.apply(document);
    }
}

/// A provider as the compositor's trait, built from a registered vtable and not yet in a slot.
enum Provider {
    Wm(ForeignWm),
    Ui(ForeignUi),
    Config(ForeignConfig),
}

/// Who holds each kind of provider, and who waits for it.
///
/// A reload stages the next generation of a shell while the serving one still holds its providers. The staged one's
/// registration cannot be refused `E_BUSY` (it would then have no providers once the old one retires), so it waits here and
/// takes the slot when the holder's registration ends. A registration from any other module is refused as usual.
#[derive(Default)]
struct Holders {
    held: HashMap<u32, u64>,
    waiting: Vec<WaitingProvider>,
}

/// A staged generation's registration that waits for the serving generation's to end.
struct WaitingProvider {
    owner: u64,
    kind: u32,
    cell: Cell,
    provider: Option<Provider>,
    drop_user: Option<GnoblinDropFn>,
}

/// Where a registration's provider is, once it is in a slot. Empty while it waits.
type Cell = Arc<Mutex<Option<ProviderId>>>;

static HOLDERS: LazyLock<Mutex<Holders>> = LazyLock::new(Mutex::default);

fn lock_holders() -> std::sync::MutexGuard<'static, Holders> {
    HOLDERS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Puts the provider in the slot of its kind. The provider stays in `provider` when the slot is taken.
fn install(
    slots: &ProviderSlots,
    kind: u32,
    provider: &mut Option<Provider>,
    drop_user: Option<GnoblinDropFn>,
) -> Result<ProviderId, ()> {
    match kind {
        abi::PROVIDER_WM => slots.set_window_manager(|| {
            let Some(Provider::Wm(mut foreign)) = provider.take() else {
                unreachable!("a window manager was built for this slot");
            };
            foreign.adopt(drop_user);
            Box::new(foreign)
        }),
        abi::PROVIDER_UI => slots.set_ui(|| {
            let Some(Provider::Ui(mut foreign)) = provider.take() else {
                unreachable!("a portal UI was built for this slot");
            };
            foreign.adopt(drop_user);
            Box::new(foreign)
        }),
        _ => slots.set_config(|| {
            let Some(Provider::Config(mut foreign)) = provider.take() else {
                unreachable!("a configuration observer was built for this slot");
            };
            foreign.adopt(drop_user);
            Box::new(DocumentWatcher(foreign))
        }),
    }
    .map_err(|_| ())
}

/// A provider's registration ended: its slot is free, and a staged generation of the same shell that was waiting for it takes it.
fn ended(slots: &ProviderSlots, kind: u32, owner: u64, cell: &Cell) {
    let id = cell.lock().unwrap_or_else(PoisonError::into_inner).take();
    let mut holders = lock_holders();
    let Some(id) = id else {
        // It never got the slot: forget that it waits.
        holders.waiting.retain(|waiting| !Arc::ptr_eq(&waiting.cell, cell));
        return;
    };
    slots.remove(id);
    if holders.held.get(&kind) == Some(&owner) {
        holders.held.remove(&kind);
    }
    if let Some(index) = holders.waiting.iter().position(|waiting| waiting.kind == kind) {
        let mut next = holders.waiting.remove(index);
        match install(slots, kind, &mut next.provider, next.drop_user) {
            Ok(id) => {
                *next.cell.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
                holders.held.insert(kind, next.owner);
            }
            Err(()) => holders.waiting.insert(index, next),
        }
    }
}

/// `provider_register`.
pub(crate) fn register(
    ctx: GnoblinCtx,
    kind: u32,
    vtable: *const c_void,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    register_in(&slots(), ctx, kind, vtable, user, drop_user)
}

/// [`register`] into `slots`, for a test that keeps its own.
pub(crate) fn register_in(
    slots: &ProviderSlots,
    ctx: GnoblinCtx,
    kind: u32,
    vtable: *const c_void,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    let what = "provider_register";
    let Ok(owner) = registry::owner_of(ctx, what) else {
        return 0;
    };
    // SAFETY: the table's contract: `vtable` points at the vtable of `kind`, readable by its own size, and `user` is the
    // registrant's, usable by the provider's functions from one thread at a time.
    let built = unsafe { build(kind, vtable, user) };
    let mut provider = match built {
        Ok(provider) => Some(provider),
        Err((code, message)) => {
            set_code_error(code, format!("{what}: {message}"));
            return 0;
        }
    };
    // The registration is a host registration that listens for nothing: its cleanup ends the provider, and `drop_user` is
    // not given to the host, because the provider's `user` is dropped where the provider is, on the compositor's thread.
    let cell: Cell = Arc::new(Mutex::new(None));
    let (end_slots, end_cell) = (slots.clone(), Arc::clone(&cell));
    let subscription = Subscription {
        matches: Box::new(|_| false),
        call: Arc::new(|_| gnoblin_shell_host::generation::CallbackOutcome::Handled),
        cleanup: Some(Box::new(move || ended(&end_slots, kind, owner, &end_cell))),
    };
    let Ok(reg_id) = registry::subscribe(ctx, what, subscription, core::ptr::null_mut(), None) else {
        return 0;
    };
    let mut holders = lock_holders();
    match install(slots, kind, &mut provider, drop_user) {
        Ok(id) => {
            *cell.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
            holders.held.insert(kind, owner);
            encode(KIND_REG, reg_id)
        }
        Err(()) => {
            // Taken. A staged generation of the shell that holds it is a reload in progress: it waits its turn.
            let holder = holders.held.get(&kind).copied();
            drop(holders);
            let reloading = holder.is_some_and(|holder| {
                holder != owner
                    && registry::state_of(ctx) == Some(registry::GenerationState::Staged)
                    && registry::module_path_of(holder)
                        .is_some_and(|held| registry::module_path_of(owner) == Some(held))
            });
            if reloading {
                let mut holders = lock_holders();
                // The holder may have ended while this looked: then the slot is free for it now.
                if holders.held.contains_key(&kind) {
                    holders.waiting.push(WaitingProvider {
                        owner,
                        kind,
                        cell,
                        provider,
                        drop_user,
                    });
                    return encode(KIND_REG, reg_id);
                }
                if let Ok(id) = install(slots, kind, &mut provider, drop_user) {
                    *cell.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
                    holders.held.insert(kind, owner);
                    return encode(KIND_REG, reg_id);
                }
            }
            registry::remove_id(reg_id);
            set_code_error(
                abi::E_BUSY,
                format!("{what}: a provider of kind {kind} is already registered"),
            );
            0
        }
    }
}

/// Wraps the vtable as the trait of its kind.
///
/// # Safety
/// As [`register`].
unsafe fn build(kind: u32, vtable: *const c_void, user: *mut c_void) -> Result<Provider, (i32, String)> {
    let bad = |message: String| (abi::E_ABI, message);
    match kind {
        // SAFETY: forwarded.
        abi::PROVIDER_WM => unsafe { ForeignWm::new(vtable.cast::<GnoblinWmVtable>(), user, report("window manager")) }
            .map(Provider::Wm)
            .map_err(bad),
        // SAFETY: forwarded.
        abi::PROVIDER_UI => unsafe { ForeignUi::new(vtable.cast::<GnoblinUiVtable>(), user, report("portal UI")) }
            .map(Provider::Ui)
            .map_err(bad),
        // SAFETY: forwarded.
        abi::PROVIDER_CONFIG => {
            unsafe { ForeignConfig::new(vtable.cast::<GnoblinConfigVtable>(), user, report("configuration")) }
                .map(Provider::Config)
                .map_err(bad)
        }
        other => Err((abi::E_UNSUPPORTED, format!("no provider of kind {other}"))),
    }
}

/// What a `GnoblinValue` carries, as the compositor's setting value.
///
/// # Safety
/// `raw` is null or points at a value whose pointer fields are readable for their lengths.
unsafe fn setting_value(raw: *const GnoblinValue) -> Result<SettingValue, (i32, String)> {
    // SAFETY: the caller guarantees the pointer is null or valid.
    let Some(raw) = (unsafe { raw.as_ref() }) else {
        return Err((abi::E_ERR, "a null value".to_owned()));
    };
    let bytes = || {
        if raw.len == 0 {
            Ok(&[][..])
        } else if raw.ptr.is_null() {
            Err((abi::E_ERR, "a null pointer with a nonzero length".to_owned()))
        } else {
            // SAFETY: the caller guarantees `ptr` is readable for `len` bytes.
            Ok(unsafe { std::slice::from_raw_parts(raw.ptr.cast::<u8>(), raw.len) })
        }
    };
    Ok(match raw.kind {
        abi::VALUE_BOOL => SettingValue::Bool(raw.bits != 0),
        abi::VALUE_INT => SettingValue::Int(raw.bits as i64),
        abi::VALUE_UINT => SettingValue::Uint(raw.bits),
        abi::VALUE_DOUBLE => SettingValue::Double(f64::from_bits(raw.bits)),
        abi::VALUE_TEXT => SettingValue::Text(
            std::str::from_utf8(bytes()?)
                .map_err(|_| (abi::E_ERR, "text that is not UTF-8".to_owned()))?
                .to_owned(),
        ),
        abi::VALUE_BYTES => SettingValue::Bytes(bytes()?.to_vec()),
        other => return Err((abi::E_TYPE, format!("a value of kind {other} is not a setting"))),
    })
}

/// `setting_set`.
pub(crate) fn setting_set(ctx: GnoblinCtx, key: &str, value: *const GnoblinValue) -> i32 {
    setting_set_in(&slots(), ctx, key, value)
}

/// [`setting_set`] into `slots`, for a test that keeps its own.
pub(crate) fn setting_set_in(slots: &ProviderSlots, ctx: GnoblinCtx, key: &str, value: *const GnoblinValue) -> i32 {
    let what = "setting_set";
    // Any thread: a configuration observer calls this from the thread that accepted a commit.
    if let Err(code) = registry::check_ctx(ctx, what, false) {
        return code;
    }
    // SAFETY: the table's contract passes a value readable as a `GnoblinValue`, or null.
    let value = match unsafe { setting_value(value) } {
        Ok(value) => value,
        Err((code, message)) => {
            set_code_error(code, format!("{what}: {message}"));
            return code;
        }
    };
    match slots.setting_set(key, &value) {
        Ok(()) => abi::E_OK,
        Err(SettingError::UnknownKey) => {
            set_code_error(abi::E_NO_SUCH_MEMBER, format!("{what}: no setting `{key}`"));
            abi::E_NO_SUCH_MEMBER
        }
        Err(error @ SettingError::WrongType(_)) => {
            set_code_error(abi::E_TYPE, format!("{what}: `{key}`: {error}"));
            abi::E_TYPE
        }
        Err(SettingError::Refused(why)) => {
            set_code_error(abi::E_REFUSED, format!("{what}: `{key}`: {why}"));
            abi::E_REFUSED
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gnoblin_compositor::portal::NoUi;
    use gnoblin_core_abi::GnoblinValue;
    use gnoblin_provider_wire::{export_config, export_ui, export_wm};
    use gnoblin_wm_api::NullWm;

    use super::*;
    use crate::guard::last_error;
    use crate::tests::{Gen, serial};

    /// How many `user` pointers the tests' own drop function has freed.
    static FREED: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn counting_drop(user: *mut c_void) {
        FREED.fetch_add(1, Ordering::SeqCst);
        // SAFETY: every test registers a `user` made by `export_wm`, `export_ui` or `export_config`, whose own drop frees it.
        // The three free a `Box` of their own host type, which `drop_wm` and the others know; the test keeps them in `pending`.
        PENDING.with(|pending| {
            if let Some(free) = pending.borrow_mut().remove(&(user as usize)) {
                // SAFETY: `free` is the provider's own `drop_user` for this pointer, called once.
                unsafe { free(user) };
            }
        });
    }

    thread_local! {
        /// The providers' own `drop_user` for each pointer a test registered, which `counting_drop` forwards to.
        static PENDING: std::cell::RefCell<HashMap<usize, GnoblinDropFn>> = std::cell::RefCell::new(HashMap::new());
    }

    fn wm() -> (*const c_void, *mut c_void) {
        let exported = export_wm(Box::<NullWm>::default());
        PENDING.with(|pending| pending.borrow_mut().insert(exported.user as usize, exported.drop_user));
        (core::ptr::from_ref(exported.vtable).cast(), exported.user)
    }

    fn ui() -> (*const c_void, *mut c_void) {
        let exported = export_ui(Box::new(NoUi));
        PENDING.with(|pending| pending.borrow_mut().insert(exported.user as usize, exported.drop_user));
        (core::ptr::from_ref(exported.vtable).cast(), exported.user)
    }

    fn config(heard: &Arc<Mutex<Vec<String>>>) -> (*const c_void, *mut c_void) {
        let heard = Arc::clone(heard);
        let exported = export_config(Box::new(move |document| {
            heard.lock().expect("lock").push(document.to_owned());
            Ok(())
        }));
        PENDING.with(|pending| pending.borrow_mut().insert(exported.user as usize, exported.drop_user));
        (core::ptr::from_ref(exported.vtable).cast(), exported.user)
    }

    /// Frees a provider the core refused, the way its registrant does: a refusal adopts nothing.
    fn free_unadopted(user: *mut c_void) {
        PENDING.with(|pending| {
            let free = pending
                .borrow_mut()
                .remove(&(user as usize))
                .expect("a provider this test made");
            // SAFETY: the refused registration adopted nothing, so this is the only owner of `user`.
            unsafe { free(user) };
        });
    }

    fn register_wm(slots: &ProviderSlots, ctx: GnoblinCtx) -> GnoblinReg {
        let (vtable, user) = wm();
        let reg = register_in(slots, ctx, abi::PROVIDER_WM, vtable, user, Some(counting_drop));
        if reg == 0 {
            free_unadopted(user);
        }
        reg
    }

    /// Whether the window manager slot is taken.
    fn wm_taken(slots: &ProviderSlots) -> bool {
        slots.set_window_manager(|| Box::<NullWm>::default()).is_err()
    }

    fn generation(path: &str) -> Gen {
        let generation = Gen {
            ctx: registry::new_context(path, abi::LANG_NATIVE, 0, false),
        };
        registry::activate(generation.ctx).expect("activates");
        generation
    }

    #[test]
    fn a_registered_provider_serves_until_its_generation_retires_and_is_freed_once() {
        let _serial = serial();
        let slots = ProviderSlots::new();
        let freed = FREED.load(Ordering::SeqCst);
        let generation = generation("a.so");
        assert_ne!(register_wm(&slots, generation.ctx), 0, "{}", last_error());
        assert!(wm_taken(&slots));
        drop(generation);
        assert!(!wm_taken(&slots), "the slot is free once the generation retired");
        // `wm_taken` registered a `NullWm` of its own in the free slot; the test's own provider was freed once.
        assert_eq!(FREED.load(Ordering::SeqCst), freed + 1);
    }

    #[test]
    fn a_second_provider_of_a_kind_from_another_module_is_busy_and_is_not_adopted() {
        let _serial = serial();
        let slots = ProviderSlots::new();
        let first = generation("a.so");
        let second = generation("b.so");
        assert_ne!(register_wm(&slots, first.ctx), 0);
        let freed = FREED.load(Ordering::SeqCst);
        let (vtable, user) = wm();
        assert_eq!(
            register_in(&slots, second.ctx, abi::PROVIDER_WM, vtable, user, Some(counting_drop)),
            0
        );
        assert!(last_error().starts_with("E_BUSY"), "{}", last_error());
        assert_eq!(
            FREED.load(Ordering::SeqCst),
            freed,
            "a refused provider is not adopted: its owner frees it"
        );
        free_unadopted(user);
    }

    #[test]
    fn a_staged_generation_of_the_same_shell_waits_for_the_serving_one_and_takes_the_slot_when_it_ends() {
        let _serial = serial();
        let slots = ProviderSlots::new();
        let serving = generation("shell.so");
        assert_ne!(register_wm(&slots, serving.ctx), 0);

        // The reload stages the next generation while the old one serves.
        let next = Gen {
            ctx: registry::new_context("shell.so", abi::LANG_NATIVE, 1, false),
        };
        let reg = register_wm(&slots, next.ctx);
        assert_ne!(reg, 0, "a reload's staged generation is not refused: {}", last_error());
        assert!(wm_taken(&slots));

        drop(serving);
        assert!(
            wm_taken(&slots),
            "the staged generation holds the slot as the old one ends"
        );
        registry::activate(next.ctx).expect("the next generation activates");
        drop(next);
        assert!(!wm_taken(&slots), "and frees it when it ends in turn");
    }

    #[test]
    fn the_ui_and_the_observer_are_one_each_and_the_observer_hears_the_document() {
        let _serial = serial();
        let slots = ProviderSlots::new();
        slots.commit_document("{\"a\":1}".to_owned());
        let generation = generation("a.so");
        let (vtable, user) = ui();
        assert_ne!(
            register_in(
                &slots,
                generation.ctx,
                abi::PROVIDER_UI,
                vtable,
                user,
                Some(counting_drop)
            ),
            0,
            "{}",
            last_error()
        );
        let heard = Arc::new(Mutex::new(Vec::new()));
        let (vtable, user) = config(&heard);
        assert_ne!(
            register_in(
                &slots,
                generation.ctx,
                abi::PROVIDER_CONFIG,
                vtable,
                user,
                Some(counting_drop)
            ),
            0,
            "{}",
            last_error()
        );
        assert_eq!(
            *heard.lock().expect("lock"),
            ["{\"a\":1}"],
            "the document it registered after"
        );
        slots.commit_document("{\"b\":2}".to_owned());
        assert_eq!(heard.lock().expect("lock").len(), 2);
        drop(generation);
        slots.commit_document("{\"c\":3}".to_owned());
        assert_eq!(heard.lock().expect("lock").len(), 2, "a retired observer hears nothing");
    }

    #[test]
    fn an_unknown_kind_and_a_retired_context_are_refused() {
        let _serial = serial();
        let slots = ProviderSlots::new();
        let generation = generation("a.so");
        let (vtable, user) = wm();
        assert_eq!(
            register_in(&slots, generation.ctx, 99, vtable, user, Some(counting_drop)),
            0
        );
        assert!(last_error().starts_with("E_UNSUPPORTED"), "{}", last_error());
        let ctx = generation.ctx;
        drop(generation);
        assert_eq!(
            register_in(&slots, ctx, abi::PROVIDER_WM, vtable, user, Some(counting_drop)),
            0
        );
        free_unadopted(user);
    }

    fn text(value: &str) -> GnoblinValue {
        GnoblinValue {
            size: core::mem::size_of::<GnoblinValue>() as u32,
            kind: abi::VALUE_TEXT,
            flags: 0,
            reserved: 0,
            bits: 0,
            ptr: value.as_ptr().cast(),
            len: value.len(),
        }
    }

    #[test]
    fn setting_set_names_what_went_wrong() {
        let _serial = serial();
        let slots = ProviderSlots::new();
        let generation = generation("a.so");
        let flash = text("frame");
        assert_eq!(setting_set_in(&slots, generation.ctx, "bell.flash", &flash), abi::E_OK);
        assert_eq!(
            setting_set_in(&slots, generation.ctx, "bell.flash", &text("blink")),
            abi::E_REFUSED
        );
        assert_eq!(
            setting_set_in(&slots, generation.ctx, "no.such.key", &flash),
            abi::E_NO_SUCH_MEMBER
        );
        assert_eq!(
            setting_set_in(&slots, generation.ctx, "wm.edge_tiling", &flash),
            abi::E_TYPE
        );
        assert_eq!(
            setting_set_in(&slots, generation.ctx, "bell.flash", core::ptr::null()),
            abi::E_ERR
        );
    }
}
