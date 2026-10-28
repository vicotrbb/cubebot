//! A transmit-only 16550 UART: enough for the Linux 8250 driver to probe it and write the console.
use std::io::{self, Write};

const DLAB: u8 = 0x80;
const THR_EMPTY_INTERRUPT: u8 = 0x02;
const LOOPBACK: u8 = 0x10;

#[derive(Default)]
pub struct Serial {
    ier: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    divisor: [u8; 2],
    thr_empty_pending: bool,
}

impl Serial {
    /// Writes one register and returns whether the guest's serial IRQ must be raised.
    pub fn write(&mut self, offset: u16, value: u8, out: &mut impl Write) -> io::Result<bool> {
        let dlab = self.lcr & DLAB != 0;
        match offset {
            0 | 1 if dlab => self.divisor[offset as usize] = value,
            0 => {
                if self.mcr & LOOPBACK == 0 {
                    out.write_all(&[value])?;
                }
                return Ok(self.raise_thr_empty());
            }
            1 => {
                self.ier = value & 0x0f;
                return Ok(self.raise_thr_empty());
            }
            3 => self.lcr = value,
            4 => self.mcr = value,
            7 => self.scr = value,
            _ => (),
        }
        Ok(false)
    }

    pub fn read(&mut self, offset: u16) -> u8 {
        let dlab = self.lcr & DLAB != 0;
        match offset {
            0 | 1 if dlab => self.divisor[offset as usize],
            1 => self.ier,
            // Reading IIR acknowledges a pending transmit-empty interrupt, as on hardware.
            2 if std::mem::take(&mut self.thr_empty_pending) => 0x02,
            2 => 0x01,
            3 => self.lcr,
            4 => self.mcr,
            // Transmitter always empty, no received data.
            5 => 0x60,
            6 => self.modem_status(),
            7 => self.scr,
            _ => 0,
        }
    }

    fn raise_thr_empty(&mut self) -> bool {
        self.thr_empty_pending = self.ier & THR_EMPTY_INTERRUPT != 0;
        self.thr_empty_pending
    }

    fn modem_status(&self) -> u8 {
        if self.mcr & LOOPBACK == 0 {
            return 0xb0; // DCD, DSR and CTS asserted
        }
        // Loopback wires DTR->DSR, RTS->CTS, OUT1->RI, OUT2->DCD; the driver's probe checks this.
        let mcr = self.mcr;
        (mcr & 0x01) << 5 | (mcr & 0x02) << 3 | (mcr & 0x0c) << 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_the_8250_probe_and_writes_the_console() {
        let mut serial = Serial::default();
        let mut out = Vec::new();
        // autoconfig: IER read-back and the loopback modem-status test.
        serial.write(1, 0, &mut out).unwrap();
        assert_eq!(serial.read(1) & 0x0f, 0);
        serial.write(1, 0x0f, &mut out).unwrap();
        assert_eq!(serial.read(1) & 0x0f, 0x0f);
        serial.write(4, LOOPBACK | 0x0a, &mut out).unwrap();
        assert_eq!(serial.read(6) & 0xf0, 0x90);
        serial.write(4, 0, &mut out).unwrap();
        // Interrupt-driven transmit: each byte with THRI enabled raises the IRQ once per IIR read.
        serial.write(1, THR_EMPTY_INTERRUPT, &mut out).unwrap();
        assert_eq!(serial.read(2), 0x02);
        assert!(serial.write(0, b'o', &mut out).unwrap());
        assert_eq!(serial.read(2), 0x02);
        assert_eq!(serial.read(2), 0x01);
        serial.write(1, 0, &mut out).unwrap();
        assert!(!serial.write(0, b'k', &mut out).unwrap());
        assert_eq!(serial.read(2), 0x01);
        assert_eq!(serial.read(5), 0x60);
        assert_eq!(out, b"ok");
    }
}
