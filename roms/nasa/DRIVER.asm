DRIVER   CSECT
         LA    0,FRAME
         LE    F0,TWO
         DC    X'E4F7'
         DC    Y(QSQRT+14336)
         STE   F0,RESULT
         SVC   ENDC
ENDC     DC    H'21'
TWO      DC    X'41200000'
RESULT   DC    F'0'
FRAME    DS    9F
QSQRT    DC    Y(SQRT)
         DC    X'0E00'
         END   DRIVER
