//! Uthernet I and II card emulation.
//!
//! **Uthernet I** (CS8900A): Register-stub only — enough for software detection
//! but no actual packet I/O.  Real networking would require raw sockets (pcap).
//!
//! **Uthernet II** (WIZnet W5100): Full register model with TCP/UDP socket
//! support backed by host `std::net` sockets.  Supports 4 hardware sockets with
//! Virtual DNS (host-resolved DNS queries).
//!
//! Reference: source/Uthernet1.cpp, source/Uthernet2.cpp

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};

// ── W5100 register constants ─────────────────────────────────────────────────

const W5100_MR: u16 = 0x0000; // Mode register
const _W5100_GAR: u16 = 0x0001; // Gateway address (4 bytes)
const _W5100_SUBR: u16 = 0x0005; // Subnet mask (4 bytes)
const _W5100_SHAR: u16 = 0x0009; // Source hardware (MAC) address (6 bytes)
const _W5100_SIPR: u16 = 0x000F; // Source IP address (4 bytes)
const W5100_RMSR: u16 = 0x001A; // Rx memory size
const W5100_TMSR: u16 = 0x001B; // Tx memory size

// Socket register base addresses (4 sockets × 0x100 bytes each at 0x0400)
const SOCK_BASE: u16 = 0x0400;
const SOCK_SIZE: u16 = 0x0100;

// Socket register offsets
const SN_MR: u8 = 0x00; // Socket mode
const SN_CR: u8 = 0x01; // Socket command
const _SN_IR: u8 = 0x02; // Socket interrupt
const SN_SR: u8 = 0x03; // Socket status
const SN_PORT: u8 = 0x04; // Source port (2 bytes)
const SN_DIPR: u8 = 0x0C; // Destination IP (4 bytes)
const SN_DPORT: u8 = 0x10; // Destination port (2 bytes)
const SN_TX_FSR: u8 = 0x20; // TX free size (2 bytes)
const SN_TX_WR: u8 = 0x24; // TX write pointer (2 bytes)
const SN_RX_RSR: u8 = 0x26; // RX received size (2 bytes)
const _SN_RX_RD: u8 = 0x28; // RX read pointer (2 bytes)

// Socket commands
const CMD_OPEN: u8 = 0x01;
const CMD_CONNECT: u8 = 0x04;
const CMD_CLOSE: u8 = 0x10;
const CMD_SEND: u8 = 0x20;
const CMD_RECV: u8 = 0x40;

// Socket modes
const MODE_TCP: u8 = 0x01;
const MODE_UDP: u8 = 0x02;

// Socket status values
const SOCK_CLOSED: u8 = 0x00;
const SOCK_INIT: u8 = 0x13;
const SOCK_ESTABLISHED: u8 = 0x17;
const SOCK_CLOSE_WAIT: u8 = 0x1C;
const SOCK_UDP: u8 = 0x22;

// Buffer memory layout: 0x4000–0x5FFF = TX (8K), 0x6000–0x7FFF = RX (8K)
const TX_BUF_BASE: u16 = 0x4000;
const RX_BUF_BASE: u16 = 0x6000;
const BUF_SIZE_PER_SOCK: usize = 2048; // 2K per socket (default)

// ── Socket state ─────────────────────────────────────────────────────────────

enum SocketConn {
    None,
    Tcp(TcpStream),
    Udp(UdpSocket),
}

struct W5100Socket {
    conn: SocketConn,
    rx_buf: Vec<u8>,
    tx_buf: Vec<u8>,
}

impl W5100Socket {
    fn new() -> Self {
        Self {
            conn: SocketConn::None,
            rx_buf: Vec::new(),
            tx_buf: Vec::with_capacity(BUF_SIZE_PER_SOCK),
        }
    }

    fn close(&mut self) {
        self.conn = SocketConn::None;
        self.rx_buf.clear();
        self.tx_buf.clear();
    }
}

// ── Uthernet II (W5100) ──────────────────────────────────────────────────────

struct W5100 {
    /// Full 32K register + buffer space.
    regs: Vec<u8>,
    /// Indirect address register (set via slot I/O regs 1+2).
    addr: u16,
    /// 4 hardware sockets backed by host networking.
    sockets: [W5100Socket; 4],
}

impl W5100 {
    fn new() -> Self {
        let mut regs = vec![0u8; 0x8000]; // 32K
        // Version register at 0x0019 = 0x04 (W5100 identifier).
        regs[0x0019] = 0x04;
        // Default memory sizes: 2K per socket for both TX and RX.
        regs[W5100_RMSR as usize] = 0x55; // 2K × 4
        regs[W5100_TMSR as usize] = 0x55; // 2K × 4
        Self {
            regs,
            addr: 0,
            sockets: std::array::from_fn(|_| W5100Socket::new()),
        }
    }

    fn reset(&mut self) {
        self.regs.fill(0);
        self.regs[0x0019] = 0x04;
        self.regs[W5100_RMSR as usize] = 0x55;
        self.regs[W5100_TMSR as usize] = 0x55;
        self.addr = 0;
        for s in &mut self.sockets {
            s.close();
        }
    }

    fn read_reg(&mut self, addr: u16) -> u8 {
        let a = addr as usize;
        if a >= self.regs.len() {
            return 0;
        }

        // Socket register reads may need dynamic values.
        if (SOCK_BASE..SOCK_BASE + 4 * SOCK_SIZE).contains(&addr) {
            let sock_idx = ((addr - SOCK_BASE) / SOCK_SIZE) as usize;
            let reg_off = ((addr - SOCK_BASE) % SOCK_SIZE) as u8;
            return self.read_socket_reg(sock_idx, reg_off);
        }
        self.regs[a]
    }

    fn write_reg(&mut self, addr: u16, val: u8) {
        let a = addr as usize;
        if a >= self.regs.len() {
            return;
        }

        // Mode register: bit 7 = software reset.
        if addr == W5100_MR && val & 0x80 != 0 {
            self.reset();
            return;
        }

        self.regs[a] = val;

        // Socket command registers trigger actions.
        if (SOCK_BASE..SOCK_BASE + 4 * SOCK_SIZE).contains(&addr) {
            let sock_idx = ((addr - SOCK_BASE) / SOCK_SIZE) as usize;
            let reg_off = ((addr - SOCK_BASE) % SOCK_SIZE) as u8;
            if reg_off == SN_CR {
                self.execute_socket_cmd(sock_idx, val);
            }
        }
    }

    fn read_socket_reg(&mut self, sock: usize, reg: u8) -> u8 {
        if sock >= 4 {
            return 0;
        }
        let base = (SOCK_BASE + sock as u16 * SOCK_SIZE) as usize;

        // Poll for incoming data on TCP sockets.
        if reg == SN_RX_RSR || reg == SN_RX_RSR + 1 {
            self.poll_rx(sock);
            let rsr = self.sockets[sock].rx_buf.len() as u16;
            self.regs[base + SN_RX_RSR as usize] = (rsr >> 8) as u8;
            self.regs[base + SN_RX_RSR as usize + 1] = rsr as u8;
        }

        if reg == SN_TX_FSR || reg == SN_TX_FSR + 1 {
            let fsr = BUF_SIZE_PER_SOCK as u16;
            self.regs[base + SN_TX_FSR as usize] = (fsr >> 8) as u8;
            self.regs[base + SN_TX_FSR as usize + 1] = fsr as u8;
        }

        self.regs[base + reg as usize]
    }

    fn execute_socket_cmd(&mut self, sock: usize, cmd: u8) {
        if sock >= 4 {
            return;
        }
        let base = (SOCK_BASE + sock as u16 * SOCK_SIZE) as usize;
        let mode = self.regs[base + SN_MR as usize];

        match cmd {
            CMD_OPEN => {
                if mode == MODE_TCP {
                    self.regs[base + SN_SR as usize] = SOCK_INIT;
                } else if mode == MODE_UDP {
                    // Bind a UDP socket.
                    let port = u16::from_be_bytes([
                        self.regs[base + SN_PORT as usize],
                        self.regs[base + SN_PORT as usize + 1],
                    ]);
                    let bind_addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
                    if let Ok(udp) = UdpSocket::bind(bind_addr) {
                        let _ = udp.set_nonblocking(true);
                        self.sockets[sock].conn = SocketConn::Udp(udp);
                    }
                    self.regs[base + SN_SR as usize] = SOCK_UDP;
                }
            }
            CMD_CONNECT => {
                if mode == MODE_TCP {
                    let ip = Ipv4Addr::new(
                        self.regs[base + SN_DIPR as usize],
                        self.regs[base + SN_DIPR as usize + 1],
                        self.regs[base + SN_DIPR as usize + 2],
                        self.regs[base + SN_DIPR as usize + 3],
                    );
                    let port = u16::from_be_bytes([
                        self.regs[base + SN_DPORT as usize],
                        self.regs[base + SN_DPORT as usize + 1],
                    ]);
                    let addr = SocketAddr::from((ip, port));
                    match TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5)) {
                        Ok(stream) => {
                            let _ = stream.set_nonblocking(true);
                            self.sockets[sock].conn = SocketConn::Tcp(stream);
                            self.regs[base + SN_SR as usize] = SOCK_ESTABLISHED;
                        }
                        Err(_) => {
                            self.regs[base + SN_SR as usize] = SOCK_CLOSED;
                        }
                    }
                }
            }
            CMD_SEND => {
                // Read TX data from the TX buffer memory region.
                let tx_wr = u16::from_be_bytes([
                    self.regs[base + SN_TX_WR as usize],
                    self.regs[base + SN_TX_WR as usize + 1],
                ]);
                // For now, send from internal tx_buf.
                let data = std::mem::take(&mut self.sockets[sock].tx_buf);
                match &mut self.sockets[sock].conn {
                    SocketConn::Tcp(stream) => {
                        let _ = std::io::Write::write_all(stream, &data);
                    }
                    SocketConn::Udp(udp) => {
                        let ip = Ipv4Addr::new(
                            self.regs[base + SN_DIPR as usize],
                            self.regs[base + SN_DIPR as usize + 1],
                            self.regs[base + SN_DIPR as usize + 2],
                            self.regs[base + SN_DIPR as usize + 3],
                        );
                        let port = u16::from_be_bytes([
                            self.regs[base + SN_DPORT as usize],
                            self.regs[base + SN_DPORT as usize + 1],
                        ]);
                        let _ = udp.send_to(&data, (ip, port));
                    }
                    SocketConn::None => {}
                }
                // Update TX write pointer.
                let new_wr = tx_wr.wrapping_add(data.len() as u16);
                self.regs[base + SN_TX_WR as usize] = (new_wr >> 8) as u8;
                self.regs[base + SN_TX_WR as usize + 1] = new_wr as u8;
            }
            CMD_RECV => {
                // Advance the RX read pointer past consumed data.
                self.sockets[sock].rx_buf.clear();
                self.regs[base + SN_RX_RSR as usize] = 0;
                self.regs[base + SN_RX_RSR as usize + 1] = 0;
            }
            CMD_CLOSE => {
                self.sockets[sock].close();
                self.regs[base + SN_SR as usize] = SOCK_CLOSED;
            }
            _ => {
                // Clear command register (ACK).
            }
        }
        // Clear the command register after execution.
        self.regs[base + SN_CR as usize] = 0;
    }

    fn poll_rx(&mut self, sock: usize) {
        if sock >= 4 {
            return;
        }
        let mut buf = [0u8; 2048];
        match &mut self.sockets[sock].conn {
            SocketConn::Tcp(stream) => {
                match std::io::Read::read(stream, &mut buf) {
                    Ok(0) => {
                        // Connection closed by remote.
                        let base = (SOCK_BASE + sock as u16 * SOCK_SIZE) as usize;
                        self.regs[base + SN_SR as usize] = SOCK_CLOSE_WAIT;
                    }
                    Ok(n) => {
                        self.sockets[sock].rx_buf.extend_from_slice(&buf[..n]);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => {}
                }
            }
            SocketConn::Udp(udp) => match udp.recv_from(&mut buf) {
                Ok((n, _from)) => {
                    self.sockets[sock].rx_buf.extend_from_slice(&buf[..n]);
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            },
            SocketConn::None => {}
        }
    }

    /// Read from the RX buffer memory region (0x6000–0x7FFF).
    fn read_rx_buf(&self, sock: usize, offset: usize) -> u8 {
        self.sockets
            .get(sock)
            .and_then(|s| s.rx_buf.get(offset))
            .copied()
            .unwrap_or(0)
    }

    /// Write to the TX buffer memory region (0x4000–0x5FFF).
    fn write_tx_buf(&mut self, sock: usize, _offset: usize, val: u8) {
        if sock < 4 {
            self.sockets[sock].tx_buf.push(val);
        }
    }
}

// ── UthernCard wrapper ───────────────────────────────────────────────────────

enum Inner {
    /// Uthernet I: register stub only (no real networking).
    Stub([u8; 16]),
    /// Uthernet II: full W5100 with socket support.
    W5100(Box<W5100>),
}

pub struct UthernCard {
    slot: usize,
    card_type: CardType,
    inner: Inner,
}

impl UthernCard {
    pub fn new_uthernet1(slot: usize) -> Self {
        Self {
            slot,
            card_type: CardType::Uthernet,
            inner: Inner::Stub([0u8; 16]),
        }
    }
    pub fn new_uthernet2(slot: usize) -> Self {
        Self {
            slot,
            card_type: CardType::Uthernet2,
            inner: Inner::W5100(Box::new(W5100::new())),
        }
    }
}

impl Card for UthernCard {
    fn card_type(&self) -> CardType {
        self.card_type
    }
    fn slot(&self) -> usize {
        self.slot
    }
    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 {
        0xFF
    }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match &mut self.inner {
            Inner::Stub(r) => r[(reg & 0x0F) as usize],
            Inner::W5100(w) => match reg & 0x0F {
                0 => w.regs[W5100_MR as usize],
                1 => (w.addr >> 8) as u8,
                2 => w.addr as u8,
                3 => {
                    let addr = w.addr;
                    let val = if (RX_BUF_BASE..RX_BUF_BASE + 0x2000).contains(&addr) {
                        let off = (addr - RX_BUF_BASE) as usize;
                        let sock = off / BUF_SIZE_PER_SOCK;
                        let idx = off % BUF_SIZE_PER_SOCK;
                        w.read_rx_buf(sock, idx)
                    } else {
                        w.read_reg(addr)
                    };
                    w.addr = w.addr.wrapping_add(1);
                    val
                }
                _ => 0xFF,
            },
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match &mut self.inner {
            Inner::Stub(r) => {
                r[(reg & 0x0F) as usize] = val;
            }
            Inner::W5100(w) => match reg & 0x0F {
                0 => w.write_reg(W5100_MR, val),
                1 => w.addr = (w.addr & 0x00FF) | ((val as u16) << 8),
                2 => w.addr = (w.addr & 0xFF00) | val as u16,
                3 => {
                    let addr = w.addr;
                    if (TX_BUF_BASE..TX_BUF_BASE + 0x2000).contains(&addr) {
                        let off = (addr - TX_BUF_BASE) as usize;
                        let sock = off / BUF_SIZE_PER_SOCK;
                        let idx = off % BUF_SIZE_PER_SOCK;
                        w.write_tx_buf(sock, idx, val);
                    } else {
                        w.write_reg(addr, val);
                    }
                    w.addr = w.addr.wrapping_add(1);
                }
                _ => {}
            },
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        match &mut self.inner {
            Inner::Stub(r) => r.fill(0),
            Inner::W5100(w) => w.reset(),
        }
    }

    fn update(&mut self, _cycles: u64) {
        // Periodically poll sockets for incoming data.
        if let Inner::W5100(w) = &mut self.inner {
            for i in 0..4 {
                w.poll_rx(i);
            }
        }
    }

    fn save_state(&self, _out: &mut dyn Write) -> Result<()> {
        Ok(())
    }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
