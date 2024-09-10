use anchor_lang::prelude::*;

#[derive(AnchorDeserialize)]
pub enum MarinadeFinanceInstruction {
    Initialize,
    ChangeAuthority,
    AddValidator,
    RemoveValidator,
    SetValidatorScore,
    ConfigValidatorSystem,
    Deposit,
    DepositStakeAccount,
    LiquidUnstake,
    AddLiquidity,
    RemoveLiquidity,
    ConfigLp,
    ConfigMarinade,
    OrderUnstake,
    Claim,
    StakeReserve,
    UpdateActive,
    UpdateDeactivated,
    DeactivateStake,
    EmergencyUnstake,
    PartialUnstake,
    MergeStakes,
    Redelegate,
    Pause,
    Resume,
    WithdrawStakeAccount,
    ReallocValidatorList,
    ReallocStakeList,
}