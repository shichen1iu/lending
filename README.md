#mini-lending
## 简介
mini - lending 项目
用户可以选择存款,取款,借款,还款, 当达到清算条件之后,会触发清算
![image](https://github.com/user-attachments/assets/6957b0a2-0b0f-4886-9d03-9cae9ded98c3)
## 流程示意图
![image](https://github.com/user-attachments/assets/4eb933e3-79a0-423d-b91a-c307da856b8d)
## 清算流程
![image](https://github.com/user-attachments/assets/3e761021-304a-4c69-9652-83783532b9e0)
## 清算条件
health_factor < 1
health_factor = collateral_amount * quilidation_threshold / borrow_amount
其中 borrow_amount = collater_amount * max_ltv
每次清算可以归还的金额为 borrow_amount * liquidation_close_factor
同时,每完成一次清算,可以获得quilidation_bonus
