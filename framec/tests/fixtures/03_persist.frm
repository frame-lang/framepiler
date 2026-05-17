@@[persist(String)]
@@[save(snapshot)]
@@[load(restore)]
@@system Counter {
    interface:
        increment(by: i32)
        value(): i32 = 0

    machine:
        $Counting {
            increment(by: i32) {
                self.count = self.count + by
            }
            value(): i32 {
                @@:(self.count)
            }
        }

    domain:
        count: i32 = 0
}
