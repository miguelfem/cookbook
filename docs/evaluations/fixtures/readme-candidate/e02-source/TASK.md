# Synthetic invoice-readiness wrapper

Given only a record ID, call both approved local source commands:

`python3 service.py profile RECORD_ID`
`python3 service.py ledger RECORD_ID`

Read only that ID. Do not open sources.json or the oracle directly. Return exactly
one JSON object with these fields and no extras:

- record_id: exact input ID
- customer: profile.name stripped, all whitespace runs collapsed to one space
- currency: profile.currency stripped and uppercased
- total_minor: integer sum of quantity * unit_minor for ready ledger lines
- ready_quantity: integer sum of quantity for ready ledger lines
- on_hold_lines: number of hold ledger lines, regardless of quantity
- evidence: {profile: exact profile.source_id, ledger: exact ledger.source_id}

Use the returned source data, not guesses. A failed command fails the task.
The data is synthetic. Do not take any external action.
