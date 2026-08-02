LAZASM   CSECT
         LFXI  1,7
         AHI   1,35
         STH   1,ANSWER
         LH    2,PATT
         STH   2,WITNESS
         SVC   DONE
DONE     DC    H'21'
ANSWER   DC    H'0'
WITNESS  DC    H'0'
PATT     DC    H'23130'
         END   LAZASM
