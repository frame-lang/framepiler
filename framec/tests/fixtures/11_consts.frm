@@system Consts(step: i32 = 5, limit: i32 = 20) {
    interface:
        tick()
        get_count(): i32

    machine:
        $Running {
            tick() {
                self.count = self.count + self.step
                if self.count >= self.limit {
                    self.count = 0
                }
            }
            get_count(): i32 { @@:(self.count) }
        }

    domain:
        step: i32 = 5
        limit: i32 = 20
        count: i32 = 0
}
