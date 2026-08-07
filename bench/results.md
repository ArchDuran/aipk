# Provenance benchmark — vanilla RAG vs strict-render

Model: `llama3.2:3b` · corpus: fictional (3 docs) · verifier: `aipk verify`

| # | type | question | vanilla cov | strict cov |
|---|------|----------|-------------|------------|
| 1 | in | Who founded Meridian Robotics and when? | 1.0 | 1.0 |
| 2 | in | What is the payload capacity of the Frostline F4? | 1.0 | 1.0 |
| 3 | in | How long does a full charge of the F4 take? | 1.0 | 1.0 |
| 4 | in | What is the minimum charge level for the F4 to start a missi | 1.0 | 1.0 |
| 5 | in | Which path planning system does the Frostline F4 use? | 1.0 | 1.0 |
| 6 | in | What is the emergency stop distance of the F4 at full loaded | 1.0 | 1.0 |
| 7 | in | What is the standard warranty period for Meridian robots and | 0.0 | 1.0 |
| 8 | in | When must an on-call engineer acknowledge an incident? | 1.0 | 1.0 |
| 9 | in | What happens during a SEV-1 incident? | 0.6000000238418579 | 0.8333333134651184 |
| 10 | in | How long is incident telemetry retained? | 1.0 | 0.0 |
| 11 | in | What is the operating temperature range of the Frostline F4? | 0.0 | 1.0 |
| 12 | in | Who is the VP of Engineering at Meridian Robotics? | 1.0 | 1.0 |
| 13 | in | When was the Icebreaker IB-2 released? | 1.0 | 1.0 |
| 14 | in | How often do robots send telemetry during missions? | 1.0 | 0.0 |
| 15 | in | Which certification body assessed the Frostline F4? | 1.0 | 1.0 |
| 16 | out | What is the price of a Frostline F4 robot? | 0.3333333432674408 | 0.0 |
| 17 | out | How many Frostline F4 units were sold in 2025? | 0.0 | 0.0 |
| 18 | out | What is Meridian Robotics' annual revenue? | 0.0 | 0.0 |
| 19 | out | Does the Frostline F4 support outdoor operation in rain? | 0.20000000298023224 | 0.0 |
| 20 | out | Who are Meridian Robotics' main competitors? | 0.2857142984867096 | 0.0 |
| 21 | out | What CPU does the Frostline F4 use? | 0.5 | 0.0 |
| 22 | out | When will the Frostline F5 be released? | 0.0 | 0.0 |
| 23 | out | How many robots does a typical Meridian customer operate? | 0.25 | 0.0 |

## Summary

| slice | vanilla | strict-render |
|-------|---------|---------------|
| in-corpus avg coverage | 0.840 | 0.856 |
| out-of-corpus avg coverage | 0.196 | 0.000 |
| out-of-corpus refusal rate | 0/8 | 8/8 |

Coverage = доля предложений ответа, подтверждённых canonical claims.
In-corpus: выше = лучше. Out-of-corpus: показывает, галлюцинирует ли режим на незнаемом.
