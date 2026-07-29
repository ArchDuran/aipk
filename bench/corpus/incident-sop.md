# Meridian Robotics — Incident Response SOP (v3.1)

## Severity levels

- **SEV-1**: robot caused or nearly caused injury, or any fire event. Response: immediate
  fleet-wide stop of the affected site within 5 minutes, on-call engineer paged.
- **SEV-2**: robot damaged goods or infrastructure. Response: stop the affected robot,
  quarantine its mission queue, report within 4 hours.
- **SEV-3**: degraded performance without damage. Response: ticket within 24 hours.

## Escalation chain

1. Site operator triggers the incident in Meridian Console.
2. On-call engineer (rotation of 6 engineers, weekly shifts) acknowledges within 15 minutes.
3. SEV-1 incidents additionally page the VP of Engineering, Priya Nandakumar.

## Data retention

- Full telemetry of an incident robot is frozen for 180 days.
- Video from the robot's onboard cameras is retained for 30 days by default,
  extended to 180 days for SEV-1 and SEV-2.

## Post-incident

- SEV-1: root-cause analysis due in 10 business days, customer report signed by the CTO.
- SEV-2: root-cause analysis due in 15 business days.
- All SEV-1 reports are reviewed at the monthly Safety Board, chaired by the CEO.

## Firmware rollback

If an incident is suspected to be firmware-related, sites may roll back to the previous
firmware within the 90-day rollback window supported by Meridian Grid.
