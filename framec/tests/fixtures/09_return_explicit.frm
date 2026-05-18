@@system ReturnExplicit {
    interface:
        decide(score: i32): String

    machine:
        $Judging {
            decide(score: i32): String {
                if score >= 60 {
                    @@:return("pass")
                }
                @@:return("fail")
            }
        }
}
