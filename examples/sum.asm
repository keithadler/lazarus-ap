; Sum an array of fullwords, tally down a counter, exercise shifts.
; Same program as the committed golden trace (tests/golden/sum_program.txt).

        ORG 0x40
ARR:    DC F(11),F(22),F(33),F(44),F(55)
SUMOUT: DC F(0)
TALLY:  DC H(3)

        ORG 0x100
START:  LFXI 5,0        ; running sum
        LA   1,ARR      ; array cursor in bits 0-15 of R1
        LFXI 2,5        ; element count in bits 0-15 of R2
LOOP:   L    3,0(1)     ; load element via base R1
        AR   5,3        ; sum += element
        LA   1,2(1)     ; cursor += 2 halfwords (fullword stride)
        BCT  2,LOOP
        ST   5,SUMOUT   ; = 165
TDLOOP: TD   TALLY
        BC   1,TDLOOP   ; loop while the tally is still positive
        LFXI 4,1
        SLL  4,10
        SRR  4,16
        XUL  4,4
DONE:   B    DONE       ; halt: branch to self
