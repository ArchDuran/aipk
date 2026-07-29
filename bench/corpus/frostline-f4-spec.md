# Frostline F4 — Technical Specification (rev. C, January 2026)

## Physical

- Payload capacity: 1400 kg on standard EUR pallets.
- Dimensions: 1710 × 980 × 310 mm (L×W×H), mast retracted.
- Mass: 460 kg including battery.
- Operating temperature: −30 °C to +45 °C.
- IP rating: IP54.

## Battery and charging

- Battery: 8.4 kWh LiFePO4 pack, hot-swappable.
- Runtime: 11 hours continuous operation at −25 °C.
- Full charge time: 74 minutes on a Meridian DockPoint charger.
- The F4 refuses to start a mission if charge is below 12 %.

## Navigation

- Path planning: Meridian Motion Kernel 7 (MMK-7).
- Localization: fused LiDAR + wheel odometry + UWB beacons; no magnetic tape required.
- Maximum speed: 2.2 m/s unloaded, 1.6 m/s loaded.
- Positioning accuracy: ±8 mm at pick and place points.

## Safety

- Dual 275° safety LiDARs, certified to EN ISO 13849-1 performance level d.
- Emergency stop distance: 0.9 m at full loaded speed.
- Frost sensors trigger a self-heating cycle when internal temperature drops below −32 °C.

## Connectivity

- Meridian Grid protocol over private 5G mesh; fallback to Wi-Fi 6E.
- Telemetry interval: every 250 ms during missions, every 5 s when idle.
