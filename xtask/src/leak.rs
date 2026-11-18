//! Confere que o log do kernel não entrega endereço do kernel (SDD 09). Vale do primeiro
//! `[rustos]` em diante, com o corpo do pânico e a saída do shell; antes dele, o firmware
//! e o Limine imprimem o que quiserem.

/// Primeiro endereço da metade alta canônica (4 níveis): igual a `HIGH_HALF` de `core/hal`.
const HIGH_HALF: u64 = 0xffff_8000_0000_0000;
/// Do último 64 KiB em diante é `-1` ou `u64::MAX`, não endereço (igual ao de `core/hal`).
const CEILING: u64 = 0xffff_ffff_ffff_0000;
/// O primeiro literal `0x…` de endereço do kernel em `log`, do primeiro `[rustos]` em diante.
pub fn find_kernel_address(log: &str) -> Option<&str> {
    address_in(&log[log.find("[rustos]")?..])
}

fn address_in(text: &str) -> Option<&str> {
    let mut rest = text;
    while let Some(at) = rest.find("0x") {
        let literal = &rest[at..];
        let digits = literal[2..]
            .bytes()
            .take_while(u8::is_ascii_hexdigit)
            .count();
        let value = u64::from_str_radix(&literal[2..2 + digits], 16).ok();
        if value.is_some_and(|v| (HIGH_HALF..CEILING).contains(&v)) {
            return Some(&literal[..2 + digits]);
        }
        rest = &literal[2..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kernel_address_in_a_kernel_line_is_found() {
        let log = "[rustos] fault: page fault at 0xffffffff9b7a5000 (write)\n";
        assert_eq!(find_kernel_address(log), Some("0xffffffff9b7a5000"));
        let direct = "  [rustos] vm: direct map at 0xffff846840000000 in 12 pages\n";
        assert_eq!(find_kernel_address(direct), Some("0xffff846840000000"));
    }

    #[test]
    fn the_address_can_come_after_other_literals() {
        let log = "[rustos] mask 0xf then 0xffffffff80001000\n";
        assert_eq!(find_kernel_address(log), Some("0xffffffff80001000"));
    }

    #[test]
    fn what_is_not_an_address_does_not_fail_the_boot() {
        for line in [
            "[rustos] cpus: FAILED mask 0xff",
            "[rustos] x: 0xffffffffffffffff",
            "[rustos] x: 0x00000000000000ff",
            "[rustos] x: 0x7fff12345678",
            "[rustos] x: 0xffff",
            "[rustos] kernel+0xffffffff",
            "[rustos] kaslr: on, image slid by up to 18 bits",
        ] {
            assert_eq!(find_kernel_address(line), None, "{line}");
        }
    }

    #[test]
    fn the_body_of_a_panic_on_the_next_line_is_checked() {
        let log =
            "[rustos] panic: panicked at k.rs:1:2:\r\nheap: dealloc de 0xffffffff9b7a7000\r\n";
        assert_eq!(find_kernel_address(log), Some("0xffffffff9b7a7000"));
        let shell = "[rustos] boot ok\r\nrustos ~ % mem\r\nbase 0xffff846840000000\r\n";
        assert_eq!(find_kernel_address(shell), Some("0xffff846840000000"));
    }

    #[test]
    fn leading_zeros_do_not_hide_an_address() {
        let log = "[rustos] x 0x00000000ffffffff9b7a5000\n";
        assert_eq!(find_kernel_address(log), Some("0x00000000ffffffff9b7a5000"));
    }

    #[test]
    fn lines_from_the_firmware_are_not_checked() {
        let log = "BdsDxe: 0xffffffff9b7a5000\n[rustos] boot ok\n";
        assert_eq!(find_kernel_address(log), None);
    }

    #[test]
    fn a_log_line_after_a_serial_escape_is_still_checked() {
        let log = "\x1b[2J[rustos] pc 0xffffffff9b7a5000\r\n";
        assert_eq!(find_kernel_address(log), Some("0xffffffff9b7a5000"));
    }
}
