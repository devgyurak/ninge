// SPDX-License-Identifier: Apache-2.0
//! The fake stub's own breakpoint and resume rules, which the diff and CLI tests rely on: `Z0`
//! records an address, `z0` removes it, and `c` runs to the next scripted state after the current
//! one whose r15 is a breakpoint address, else to the last state. These pin the fake, not the
//! hardware; the packets are the public GDB remote protocol's (`Z0,addr,kind`, `z0,addr,kind`, `c`).

use ninge_core::trace::{Opcode, TraceCpu, TraceRecord};

use super::fake::{Fake, State, script_of};
use crate::oracle::{GdbClient, Stop};

const A: u32 = 0x0200_0000;
const B: u32 = 0x0200_0004;
const C: u32 = 0x0200_0008;
const D: u32 = 0x0200_000C;

fn client(pcs: &[u32]) -> GdbClient<Fake> {
    let (fake, _) = Fake::new(pcs.iter().map(|pc| State::at(*pc)).collect());
    GdbClient::new(fake, Vec::new())
}

/// r15 after setting breakpoints at `set`, removing `cleared`, and resuming `resumes` times.
fn pc_after(pcs: &[u32], set: &[u32], cleared: &[u32], resumes: usize) -> u32 {
    let mut client = client(pcs);
    set.iter()
        .for_each(|addr| client.set_breakpoint(*addr, 4).expect("Z0 is accepted"));
    cleared
        .iter()
        .for_each(|addr| client.clear_breakpoint(*addr, 4).expect("z0 is accepted"));
    (0..resumes).for_each(|_| {
        assert_eq!(client.resume().expect("c is answered"), Stop::Signal(5));
    });
    client.registers().expect("g is answered")[15]
}

#[test]
fn given_a_breakpoint_in_the_script_when_resumed_then_the_stub_stops_there() {
    assert_eq!(pc_after(&[A, B, C, D], &[C], &[], 1), C);
}

#[test]
fn given_no_breakpoint_when_resumed_then_the_stub_stops_at_the_last_state() {
    assert_eq!(pc_after(&[A, B, C, D], &[], &[], 1), D);
}

#[test]
fn given_a_breakpoint_that_was_removed_when_resumed_then_the_stub_stops_at_the_last_state() {
    assert_eq!(pc_after(&[A, B, C, D], &[C], &[C], 1), D);
}

#[test]
fn given_two_breakpoints_when_resumed_once_then_the_stub_stops_at_the_earlier_one() {
    assert_eq!(pc_after(&[A, B, C, D], &[C, B], &[], 1), B);
}

#[test]
fn given_two_breakpoints_when_resumed_twice_then_the_stub_stops_at_the_later_one() {
    assert_eq!(pc_after(&[A, B, C, D], &[B, C], &[], 2), C);
}

#[test]
fn given_the_current_state_is_a_breakpoint_when_resumed_then_the_stub_does_not_stay() {
    // The script starts at A, which is a breakpoint; only states after the current one count.
    assert_eq!(pc_after(&[A, B, C, D], &[A, C], &[], 1), C);
}

#[test]
fn given_a_breakpoint_behind_the_current_state_when_resumed_then_the_stub_stops_at_the_last_state()
{
    assert_eq!(pc_after(&[A, B, C, D], &[B], &[], 2), D);
}

#[test]
fn given_a_breakpoint_visited_twice_when_resumed_twice_then_the_stub_stops_at_each_visit() {
    // r0 numbers the states, so the two visits of B are told apart.
    let script = vec![
        State::at(A).reg(0, 0),
        State::at(B).reg(0, 1),
        State::at(A).reg(0, 2),
        State::at(B).reg(0, 3),
        State::at(D).reg(0, 4),
    ];
    let (fake, _) = Fake::new(script);
    let mut client = GdbClient::new(fake, Vec::new());
    client.set_breakpoint(B, 4).unwrap();
    let first = (client.resume().unwrap(), client.registers().unwrap()[0]);
    let second = (client.resume().unwrap(), client.registers().unwrap()[0]);
    let stops = vec![first, second];
    assert_eq!(stops, vec![(Stop::Signal(5), 1), (Stop::Signal(5), 3)]);
}

#[test]
fn given_a_malformed_breakpoint_packet_when_sent_then_the_stub_answers_an_error() {
    let mut client = client(&[A]);
    assert_eq!(client.transact(b"Z0,zz,4").unwrap(), b"E01".to_vec());
}

#[test]
fn given_a_breakpoint_of_another_type_when_sent_then_the_stub_answers_an_error() {
    // Only software breakpoints (type 0) are simulated.
    let mut client = client(&[A]);
    assert_eq!(client.transact(b"Z1,200,4").unwrap(), b"E01".to_vec());
}

#[test]
fn given_a_thumb_record_when_made_a_state_then_the_stub_serves_its_two_opcode_bytes() {
    let mut regs = [0; 16];
    regs[3] = 7;
    regs[15] = A;
    let record = TraceRecord {
        cpu: TraceCpu::Arm9,
        step: 0,
        opcode: Opcode::Thumb(0xDF05),
        regs,
        cpsr: 0x0000_003F,
    };
    let (fake, _) = Fake::new(script_of(&[record]));
    let mut client = GdbClient::new(fake, Vec::new());
    assert_eq!(
        (
            client.read_memory(A, 2).unwrap(),
            client.registers().unwrap()[3],
            client.registers().unwrap()[16],
        ),
        (vec![0x05, 0xDF], 7, 0x3F)
    );
}

#[test]
fn given_an_arm_record_when_made_a_state_then_the_stub_serves_its_four_opcode_bytes() {
    let mut regs = [0; 16];
    regs[15] = B;
    let record = TraceRecord {
        cpu: TraceCpu::Arm9,
        step: 0,
        opcode: Opcode::Arm(0xE3A0_0055),
        regs,
        cpsr: 0x0000_001F,
    };
    let (fake, _) = Fake::new(script_of(&[record]));
    let mut client = GdbClient::new(fake, Vec::new());
    assert_eq!(
        client.read_memory(B, 4).unwrap(),
        vec![0x55, 0x00, 0xA0, 0xE3]
    );
}
