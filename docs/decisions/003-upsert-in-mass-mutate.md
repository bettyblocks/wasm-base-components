# Upsert in a Mass Mutate: placeholder ID now, find the records at apply

## Status

**Accepted.** Implemented in the `data-api` component 3.2.0.

The comparison of all options is in
[upsert-in-mass-mutate-decision-matrix.xlsx](upsert-in-mass-mutate-decision-matrix.xlsx).

## Context

A Mass Mutate collects the create, update and delete mutations of its steps in a capture. The
`data-api` component sends them to the Data API in batches when the action applies the capture.
A step that creates a record gets a negative placeholder ID. The component replaces the
placeholder with a reserved ID before it sends the batch.

The capture did not know upsert. The component sent an upsert to the Data API immediately. This
caused three problems:

- The upsert went to the database before the earlier mutations of the Mass Mutate.
- Placeholder IDs in the input of the upsert went to the Data API without a change.
- When the action discarded the capture, the upsert stayed in the database.

Four facts from the Data API and BettyQL limit the solution. We tested each fact on MariaDB 10.2
with `utf8_unicode_ci`:

1. **No `id` together with `uniqueBy`.** With `uniqueBy: ["email"]`, the Data API finds the
   record by `email` and takes its ID from that record. If the row also contains `id`, MySQL
   gets the ID two times and returns `(1110) Column 'id' specified twice`.
2. **A known record by ID fails the uniqueness check.** For example, customer 7 has the email
   `ann@x.com`. We send `upsertManyCustomer(input: [{id: 7, email: "ann@x.com", name: "Annie"}])`
   without `uniqueBy`. Before the write, the Data API counts the records with the email
   `ann@x.com`. It finds one record: customer 7. The check does not know that customer 7 is the
   record of the write. Thus it rejects the write with `'ann@x.com' already exists`. With
   `uniqueBy: ["email"]`, the check permits one record with that email. Then customer 7 does not
   conflict with itself.
3. **The same fields in each row.** All rows in one `upsertMany` must have the same fields. If
   one row has `name` and one row does not, the Data API rejects the request.
4. **MySQL ignores case and trailing spaces.** MySQL compares `Ann@x.com` and `ann@x.com ` as
   the same value.

For a found record, rules 1 and 2 tell us to use `uniqueBy` and to send no `id`. Rules 3 and 4
tell us to split the batches by fields and to compare values without case.

## Decision

We use option C: the upsert gets a placeholder ID now, and the component finds the records when
the action applies the capture.

- **Capture.** The upsert gets a negative placeholder ID, the same as a create. The step gets
  the placeholder immediately, so a later step can refer to the record. Two upserts with the
  same unique value get the same placeholder. The component compares text values without case
  and without trailing spaces.
- **Apply.** The component sends one query with an `in` filter for each model and unique
  field. The Data API returns a maximum of 200 records for each query. Thus the component
  splits the values into groups of 200 or less, and sends one query for each group. For
  example, 450 values cause 3 queries: 200, 200 and 50 values. One value finds a maximum of one
  record, because the field is unique. If the field is not unique in the database, one value
  can find more than one record. Then the Data API can omit records, and the last record that
  the component reads is used.
  - If the record exists, the placeholder gets the ID of that record. The component sends the
    row in an `upsertMany` with `uniqueBy` and without `id`.
  - If the record does not exist, the placeholder gets a reserved ID. The component sends the
    row in an `upsertMany` with that ID.
- **Batches.** One Mass Mutate can contain upserts for many models. The component sends one
  request for each model, unique field and set of fields. For example, upserts of `Customer`
  and `Order` in one Mass Mutate cause two requests in the same apply. The component merges
  upserts of the same record into one row. The later value replaces the earlier value.
- **Order.** The capture does not keep the order of the steps. It sends the creates and updates,
  then the upserts, then the deletes. Only the order of an update and the upserts before it is
  kept. See [Order of the mutations](#order-of-the-mutations).
- **Upsert by ID.** An upsert with `uniqueBy: ["id"]` is an update by ID. The component keeps it
  with the updates, and applies the same rules as for an update. Thus an upsert by ID of an
  upserted record goes into the row of that upsert.
- **No `id` in the input of an upsert by unique value.** The component returns an error when
  the input of an upsert has `id` and `uniqueBy` names a different field. The placeholder ID
  replaces the `id` of the input. Thus the component cannot use the `id` of the step. The
  Data API also rejects `id` together with `uniqueBy` when the record exists (fact 1). In
  `actions-js`, the upsert did not go into the capture. The Data API got the `id` without a
  change.
- **Nested captures.** An inner capture can apply its upserts before the outer capture. After
  that apply, the records of these upserts are in the database. The component then marks their
  placeholders as found records. An outer upsert with the same unique value gets the same
  placeholder. The component sends it with `uniqueBy` and without `id` (fact 2).
- **Discard.** When the action discards a capture, the component forgets the unique values of
  its upserts. The next apply does not look them up. A later upsert of the same value gets a
  new placeholder. The component keeps a value when an open capture also upserts it. A
  discarded placeholder still gets a reserved ID, because the next placeholders must keep their
  position in the list. The component does not use that ID.

Only the `data-api` component changes. The `crud` component, the native upsert step, the
actions-compiler, the Data API and BettyQL do not change.

## Order of the mutations

The capture does not keep the order of the steps. When the action applies the capture, the
component sends the requests in this order:

| # | Requests | One request for each |
| --- | --- | --- |
| 1 | Lookup | model, unique field and group of 200 values |
| 2 | `reserveRecords` | model, for the creates and the upserts that found no record |
| 3 | Creates and updates | model (`upsertMany`) |
| 4 | Upserts | model, `uniqueBy` or no `uniqueBy`, and set of fields (`upsertMany`) |
| 5 | Deletes | model (`deleteMany`) |

### Upserts had no order in `actions-js`

In `actions-js`, the capture held back only `create`, `update` and `delete`
(`src/worker/isolate-functions/gql-buffer/index.js`). An upsert went to the Data API during its
step, thus before all mutations that the capture held back. At apply, `actions-js` sent the
creates, then the updates, then the deletes. Its order list has a place for upserts, but the
capture never put an upsert there.

Thus upserts were never part of the order of a capture, and there is no earlier order of
upserts to keep. We send the upserts after the creates and updates. Then an upsert after an
update of the same record gives the upsert value, which follows the order of the steps. We send
the upserts before the deletes, the same as `actions-js`.

### Two steps that change the same record

| Steps | Result now | Result in `actions-js` |
| --- | --- | --- |
| An upsert, then an update by the ID that the upsert returned | The update goes into the row of the upsert. The update value is the last value. | The same. The upsert returned the real ID, and the update went at the end. |
| An update by ID, then an upsert | The upsert goes after the update. The upsert value is the last value. | The upsert went first. The update value was the last value. |
| An upsert, then an update by the real ID of the found record | The update goes into the row of the upsert. The update value is the last value. | The same. The upsert went first, and the update went at the end. |
| A delete, then an upsert | No record. The lookup finds the record, because the delete is not sent yet. The upsert changes the record, and then the delete removes it. | The same. The upsert went first, and the delete went at the end. |

Rules for these cases:

- An update by the placeholder of an upsert goes into the row of that upsert at capture.
- An update by a real ID can only go into the row of an upsert at apply, because the component
  does not know at capture which ID the upsert will find. The third row needs such a real ID, for
  example from an earlier query in the action. Thus each captured mutation keeps its position in
  the capture. At apply, an update by the real ID of a found record goes into the row of the last
  upsert of that record before the update. An update before all upserts of the record stays an
  update, and the upsert value is the last value. An upsert by ID follows the same rules.
- An update must not change the unique value of an upserted record. The component then returns
  an error, because the upsert cannot find the record by the new value. For an update by a real
  ID, the error comes at apply, before the component writes a record. This rule applies only
  when the upsert has a unique value. An upsert without a unique value is a new record, and the
  component sends it with its ID.
- An update in an inner capture of a record that an outer capture upserts is not merged. See
  [Consequences](#consequences).

## Options that we did not use

- **A. Find the record at capture.** This sends one query for each upsert. We do not want a
  round trip for each step in a Mass Mutate.
- **B. Send upserts with `uniqueBy`, without a placeholder.** The step gets no ID, so a later
  step in the same Mass Mutate cannot refer to the record.
- **D. Flush before each upsert.** A loop of upserts causes one flush for each upsert. We want
  one flush for each Mass Mutate.
- **E. Reject the upsert.** We want to support upsert.
- **F. Send with `uniqueBy` and use the returned IDs.** `upsertMany` returns the IDs in database
  order, without the unique value. The component cannot map them back to the placeholders
  without one more query. The flush must also follow the references between records, and a
  cycle has no correct order.

## Consequences

- A Mass Mutate with upserts sends one more query for each model and unique field.
- Another process can delete a found record between the query and the write. Then the upsert
  inserts the record again with a new ID, and later steps keep the old ID.
- A create and an upsert with the same unique value in one Mass Mutate both become new
  records. If the field has a unique validation, the Data API rejects the second record.
- An inner capture can update a record that an outer capture upserts. The component does not
  put that update into the row of the upsert, because the update is in a different capture.
  When the inner capture applies, the update goes to the database first. The upsert of the outer
  capture comes later, so the upsert value is the last value.

## Accepted risk: the component does not compare values the same way as MySQL

**Status:** accepted. We do not fix this now.

**Risk.** The component compares unique values without case and without trailing spaces. MySQL
compares them with the collation of the column. That collation can also ignore more differences,
for example accents (`utf8mb4_unicode_ci`). MySQL then compares `jose@x.com` and `josé@x.com` as
the same value, but the component does not.

**Effect.** Two upserts with values that are different only in such a way get two placeholders.
The lookup finds a record for one value only. The component sends the other value as a new
record:

- If the field has a unique validation, the Data API rejects that row and the apply stops. The
  batches that the component sent before that row stay in the database.
- If the field has no unique validation, the database gets a second record with the same value.

A found record is not affected when it has only one value in the Mass Mutate. MySQL itself
finds that record in the `uniqueBy` write.

**Likelihood.** Low. The risk occurs only when one Mass Mutate contains two values for a unique
field that are different only in accents or similar characters. This is rare for email
addresses and codes, and more possible for names.

**Why we do not fix it now.** The component cannot know the collation of a column, and it
cannot copy the comparison rules of MySQL exactly. The lookup returns the stored values, not
which input value MySQL found.
