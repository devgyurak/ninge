// SPDX-License-Identifier: Apache-2.0
//! The device registry: [`registry!`] is the one table of the register devices. Each entry names
//! a device once, in the group of the kind it is (`AGENTS.md` §4, "Per-CPU devices"), and the
//! macro writes everything that follows from it: the field of `Io`, its power-on state, its
//! getters, the savestate check, the port dispatch [`device_call!`] (static dispatch, no
//! `dyn`, `AGENTS.md` §2 rule 3) and the [`DeviceId`] the scheduler names a device's events by.
//!
//! The dispatch `match` has no wildcard, so a [`Port`](super::map::Port) that no entry names
//! fails to compile: registering a device cannot be forgotten halfway.
//!
//! Groups, and the CPU each one serves:
//! - `shared`: one instance both CPUs see; `Device::<ARM9>` is the CPU that asks.
//! - `arm9`: one instance only the ARM9 sees; the ARM7's access is a caller bug.
//! - `arm7`: one instance only the ARM7 sees.
//! - `each`: an independent copy per CPU, named `name9` and `name7`.
//!
//! A device's type name is its [`DeviceId`] variant, so the order of the entries is the order
//! events of the same time and slot fall in (`scheduler.md`, "Events").
//!
//! The savestate lays the fields out in the order the groups and entries have here.

/// `&$place` for `ref` and `&mut $place` for `mut`: lets [`device_call`] borrow a device either way.
macro_rules! borrow {
    (ref $place:expr) => {
        &$place
    };
    (mut $place:expr) => {
        &mut $place
    };
}

/// Defines `Io`, its constructor and getters, `Io::well_formed`, `Io::schedule_all`'s walk over
/// every device and the `device_call!` dispatch from the entries. `$d` is a literal `$`, so the
/// macro this one writes can have metavariables of its own.
macro_rules! registry {
    (
        $d:tt
        shared {$(
            $(#[$sm:meta])*
            $sf:ident $(/ $sfm:ident)?: $st:ident = $si:expr => [$sp0:path $(, $sp:path)*]
        ),* $(,)?}
        arm9 {$(
            $(#[$am:meta])*
            $af:ident $(/ $afm:ident)?: $at:ident = $ai:expr => [$ap0:path $(, $ap:path)*]
        ),* $(,)?}
        arm7 {$(
            $(#[$bm:meta])*
            $bf:ident $(/ $bfm:ident)?: $bt:ident = $bi:expr => [$bp0:path $(, $bp:path)*]
        ),* $(,)?}
        each {$(
            $(#[$em:meta])*
            $ef9:ident / $ef7:ident: $et:ident = $ei:expr => [$ep0:path $(, $ep:path)*]
        ),* $(,)?}
    ) => {
        /// The register devices. Plain data; the machine owns one next to its `Memory`.
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub struct Io {
            $( $(#[$sm])* $sf: $st, )*
            $( $(#[$am])* $af: $at, )*
            $( $(#[$bm])* $bf: $bt, )*
            $(
                $(#[$em])* $ef9: $et<true>,
                $(#[$em])* $ef7: $et<false>,
            )*
        }

        /// Event slots the registered devices have in all ([`Device::SLOTS`] summed): the
        /// scheduler sizes its queue from it, so a device that registers grows the queue.
        pub(crate) const EVENT_SLOTS: usize = 0
            $( + <$st as Device<true>>::SLOTS as usize )*
            $( + <$at as Device<true>>::SLOTS as usize )*
            $( + <$bt as Device<false>>::SLOTS as usize )*
            $( + <$et<true> as Device<true>>::SLOTS as usize
               + <$et<false> as Device<false>>::SLOTS as usize )*;

        /// Which registered device an event belongs to: one variant per registry entry, named
        /// after its type, in registration order. A pair of per-CPU copies (`each`) shares one
        /// variant; the event says which CPU's. The scheduler orders same-time events by it, so
        /// the registration order is part of the model (`scheduler.md`, "Events").
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum DeviceId {
            $( #[doc = concat!("The `", stringify!($st), "` device.")] $st, )*
            $( #[doc = concat!("The `", stringify!($at), "` device.")] $at, )*
            $( #[doc = concat!("The `", stringify!($bt), "` device.")] $bt, )*
            $( #[doc = concat!("The `", stringify!($et), "` device.")] $et, )*
        }

        impl DeviceId {
            /// Every device, in registration order.
            #[cfg(test)]
            pub(crate) const ALL: &[DeviceId] = &[
                $( Self::$st, )*
                $( Self::$at, )*
                $( Self::$bt, )*
                $( Self::$et, )*
            ];

            /// The device behind `port`. No wildcard: a port no entry names fails to compile.
            pub(crate) fn of(port: Port) -> Self {
                match port {
                    $( $sp0 $(| $sp)* => Self::$st, )*
                    $( $ap0 $(| $ap)* => Self::$at, )*
                    $( $bp0 $(| $bp)* => Self::$bt, )*
                    $( $ep0 $(| $ep)* => Self::$et, )*
                }
            }

            /// The first port of the entry: every port of one device reaches the same slots, so
            /// the first one stands for it.
            pub(crate) fn first_port(self) -> Port {
                match self {
                    $( Self::$st => $sp0, )*
                    $( Self::$at => $ap0, )*
                    $( Self::$bt => $bp0, )*
                    $( Self::$et => $ep0, )*
                }
            }

            /// How many event slots the device has ([`Device::SLOTS`]); the same for both CPUs'
            /// copies of a per-CPU device.
            pub(crate) fn slots(self) -> u8 {
                match self {
                    $( Self::$st => <$st as Device<true>>::SLOTS, )*
                    $( Self::$at => <$at as Device<true>>::SLOTS, )*
                    $( Self::$bt => <$bt as Device<false>>::SLOTS, )*
                    $( Self::$et => <$et<true> as Device<true>>::SLOTS, )*
                }
            }
        }

        // No device may declare more slots than a `SlotMask` has bits.
        const _: () = {
            $( assert!(<$st as Device<true>>::SLOTS as u32 <= SlotMask::BITS); )*
            $( assert!(<$at as Device<true>>::SLOTS as u32 <= SlotMask::BITS); )*
            $( assert!(<$bt as Device<false>>::SLOTS as u32 <= SlotMask::BITS); )*
            $( assert!(<$et<true> as Device<true>>::SLOTS as u32 <= SlotMask::BITS);
               assert!(<$et<false> as Device<false>>::SLOTS as u32 <= SlotMask::BITS); )*
        };

        impl Io {
            /// The devices in their power-on state.
            pub(crate) fn new() -> Self {
                Self {
                    $( $sf: $si, )*
                    $( $af: $ai, )*
                    $( $bf: $bi, )*
                    $( $ef9: $ei, $ef7: $ei, )*
                }
            }

            $(
                $(#[$sm])*
                pub(crate) fn $sf(&self) -> &$st {
                    &self.$sf
                }
                $(
                    pub(crate) fn $sfm(&mut self) -> &mut $st {
                        &mut self.$sf
                    }
                )?
            )*
            $(
                $(#[$am])*
                pub(crate) fn $af(&self) -> &$at {
                    &self.$af
                }
                $(
                    pub(crate) fn $afm(&mut self) -> &mut $at {
                        &mut self.$af
                    }
                )?
            )*
            $(
                $(#[$bm])*
                pub(crate) fn $bf(&self) -> &$bt {
                    &self.$bf
                }
                $(
                    pub(crate) fn $bfm(&mut self) -> &mut $bt {
                        &mut self.$bf
                    }
                )?
            )*
            $(
                $(#[$em])*
                pub(crate) fn $ef9(&self) -> &$et<true> {
                    &self.$ef9
                }
                $(#[$em])*
                pub(crate) fn $ef7(&self) -> &$et<false> {
                    &self.$ef7
                }
            )*

            /// Whether every device holds only values that exist. A savestate that decodes to
            /// anything else is corrupt (`savestate`).
            pub(crate) fn is_well_formed(&self, now: u64) -> bool {
                true
                    $( && Checked::well_formed(&self.$sf, now) )*
                    $( && Checked::well_formed(&self.$af, now) )*
                    $( && Checked::well_formed(&self.$bf, now) )*
                    $( && Checked::well_formed(&self.$ef9, now) && Checked::well_formed(&self.$ef7, now) )*
            }

            /// Queues the event of every slot of every device, as at `now`. A load calls it on
            /// an empty queue to rebuild what the savestate does not hold (`scheduler.md`,
            /// "Savestate checks"). A device with no event slots queues nothing. Each entry is
            /// asked through its first port: every port of one device reaches the same slots.
            pub(crate) fn schedule_all(&self, scheduler: &mut Scheduler, now: u64) {
                $( self.schedule_slots::<true>(scheduler, $sp0, SlotMask::MAX, now); )*
                $( self.schedule_slots::<true>(scheduler, $ap0, SlotMask::MAX, now); )*
                $( self.schedule_slots::<false>(scheduler, $bp0, SlotMask::MAX, now); )*
                $(
                    self.schedule_slots::<true>(scheduler, $ep0, SlotMask::MAX, now);
                    self.schedule_slots::<false>(scheduler, $ep0, SlotMask::MAX, now);
                )*
            }
        }

        /// The one place a [`Port`] meets its device for one CPU: calls `Device::<cpu>::$method`
        /// on the field the registry keeps for `$port`, borrowing it as `$b` says (`ref` or
        /// `mut`), and evaluates to the call's result. `$arm9` is the const generic naming the
        /// CPU that asks. A port the CPU's `Io` does not own (engine A seen from the ARM7) is a
        /// bug in the caller: it asserts in a debug build and evaluates to `$absent`.
        macro_rules! device_call {
            ($d io:expr, $d arm9:ident, $d port:expr, $d b:tt, $d method:ident($d($d arg:expr),*), $d absent:expr) => {
                match $d port {
                    $( $sp0 $(| $sp)* => {
                        Device::<$d arm9>::$d method(borrow!($d b $d io.$sf) $d(, $d arg)*)
                    } )*
                    $(
                        $ap0 $(| $ap)* if $d arm9 => {
                            Device::<true>::$d method(borrow!($d b $d io.$af) $d(, $d arg)*)
                        }
                        $ap0 $(| $ap)* => {
                            debug_assert!(false, "port not reachable through Io");
                            $d absent
                        }
                    )*
                    $(
                        $bp0 $(| $bp)* if !$d arm9 => {
                            Device::<false>::$d method(borrow!($d b $d io.$bf) $d(, $d arg)*)
                        }
                        $bp0 $(| $bp)* => {
                            debug_assert!(false, "port not reachable through Io");
                            $d absent
                        }
                    )*
                    $(
                        $ep0 $(| $ep)* if $d arm9 => {
                            Device::<true>::$d method(borrow!($d b $d io.$ef9) $d(, $d arg)*)
                        }
                        $ep0 $(| $ep)* => {
                            Device::<false>::$d method(borrow!($d b $d io.$ef7) $d(, $d arg)*)
                        }
                    )*
                }
            };
        }
    };
}
