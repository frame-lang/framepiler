@@system Actions {
    interface:
        increment(n: i32)
        get_total(): i32

    machine:
        $Counting {
            increment(n: i32) {
                _scale(n)
            }
            get_total(): i32 { @@:(self.total) }
        }

    actions:
        _scale(n: i32) {
            self.total = self.total + n * 2
        }

    domain:
        total: i32 = 0
}
